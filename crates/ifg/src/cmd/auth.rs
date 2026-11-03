//! `ifg auth …` (design doc §4.1): login with a pasted PAT, show host
//! status, forget a host, print the current token. No password login by
//! design (MFA and login rate limiting make it a bad fit for a CLI).

use std::io::IsTerminal;

use ifg_client::{Client, Secret};
use serde_json::json;

use crate::cli::{AuthCommand, Cli};
use crate::config::{self, Config, HostEntry};
use crate::context;
use crate::error::AppError;
use crate::output;

pub async fn run(command: &AuthCommand, cli: &Cli) -> Result<(), AppError> {
    match command {
        AuthCommand::Login { with_token } => login(cli, *with_token).await,
        AuthCommand::Status => status(cli).await,
        AuthCommand::Logout => logout(cli),
        AuthCommand::Token => token(cli),
        AuthCommand::GitCredential { action } => git_credential(action),
    }
}

/// Read the PAT: all of stdin with `--with-token` (pipes), a no-echo prompt
/// on a terminal, or stdin when it is redirected without the flag.
fn read_token(with_token: bool) -> Result<String, AppError> {
    if with_token {
        use std::io::Read;
        let mut buffer = String::new();
        std::io::stdin()
            .read_to_string(&mut buffer)
            .map_err(|e| AppError::Other(format!("could not read token from stdin: {e}")))?;
        Ok(buffer.trim().to_string())
    } else if std::io::stdin().is_terminal() {
        // Interactive: never echo the token (§5.3). Ctrl-C leaves the
        // terminal in its original state — rpassword restores it.
        let prompt = "Paste your personal access token (input hidden): ";
        let token = rpassword::prompt_password(prompt)
            .map_err(|e| AppError::Other(format!("could not read token: {e}")))?;
        Ok(token.trim().to_string())
    } else {
        // Piped stdin: read the first line, echo is a non-issue.
        use std::io::Read;
        let mut buffer = String::new();
        std::io::stdin()
            .read_to_string(&mut buffer)
            .map_err(|e| AppError::Other(format!("could not read token from stdin: {e}")))?;
        Ok(buffer.trim().to_string())
    }
}

async fn login(cli: &Cli, with_token: bool) -> Result<(), AppError> {
    let mut config = config::load()?;
    let (key, url) = login_host(&cli.host, &config)?;

    let token = read_token(with_token)?;
    if token.is_empty() {
        return Err(AppError::Usage("empty token".into()));
    }

    // Validate before saving (design doc §4.1).
    let client = Client::builder(&url)
        .token(Secret::new(&token))
        .user_agent(super::user_agent())
        .build()?;
    let me = client.users().me().await?;

    let uses_keyring = config.uses_keyring();
    let entry = config
        .hosts
        .entry(key.clone())
        .or_insert_with(|| HostEntry::new(url.clone()));
    entry.url = url.clone();
    entry.user = Some(me.username.clone());
    if uses_keyring {
        match crate::keyring::store(&key, &token) {
            Ok(()) => entry.token = None,
            Err(error) => {
                eprintln!(
                    "ifg: warning: keychain unavailable ({error}); storing the token in the \
                     config file instead"
                );
                entry.token = Some(token);
            }
        }
    } else {
        entry.token = Some(token);
    }
    if config.default_host.is_none() {
        config.default_host = Some(key.clone());
    }
    config::save(&config)?;

    println!("Logged in to {url} as {}", me.username);
    if uses_keyring {
        println!("Token stored in the system keychain (token_store = \"keyring\").");
    }
    Ok(())
}

/// Host for login: `--host` (name or URL) > `IRONFORGE_URL` > `default_host`.
fn login_host(host_arg: &Option<String>, config: &Config) -> Result<(String, String), AppError> {
    if let Some(host_arg) = host_arg {
        if context::is_url(host_arg) {
            let url = host_arg.trim().trim_end_matches('/').to_string();
            let key = config::host_key_for_url(&url)
                .ok_or_else(|| AppError::Other(format!("invalid host URL: {url}")))?;
            return Ok((key, url));
        }
        let entry = config.hosts.get(host_arg).ok_or_else(|| {
            AppError::Other(format!(
                "unknown host '{host_arg}'; pass a full URL instead"
            ))
        })?;
        return Ok((host_arg.clone(), entry.url.clone()));
    }
    if let Some(env_url) = context::env_url() {
        let url = env_url.trim().trim_end_matches('/').to_string();
        let key = config::host_key_for_url(&url)
            .ok_or_else(|| AppError::Other(format!("invalid IRONFORGE_URL: {url}")))?;
        return Ok((key, url));
    }
    if let Some(default) = &config.default_host {
        if let Some(entry) = config.hosts.get(default) {
            return Ok((default.clone(), entry.url.clone()));
        }
    }
    Err(AppError::Other(
        "no host to log in to. Pass --host <url>, set IRONFORGE_URL, or default_host in \
         the config."
            .into(),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    Ok,
    Denied,
    Unreachable,
}

impl Scope {
    fn glyph(self) -> &'static str {
        match self {
            Scope::Ok => "✓",
            Scope::Denied => "✗",
            Scope::Unreachable => "?",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Scope::Ok => "ok",
            Scope::Denied => "denied",
            Scope::Unreachable => "unreachable",
        }
    }
}

async fn status(cli: &Cli) -> Result<(), AppError> {
    let config = config::load()?;
    if config.hosts.is_empty() {
        eprintln!("No hosts configured. Run `ifg auth login --host <url>`.");
        return Ok(());
    }

    let mut rows = Vec::new();
    let mut json_rows = Vec::new();
    for (name, entry) in &config.hosts {
        // Build a client with the configured token (env PAT only applies to
        // the resolved host, which status does not know about).
        let mut builder = Client::builder(&entry.url)
            .user_agent(super::user_agent())
            .max_retries(0);
        if let Some(token) = &entry.token {
            builder = builder.token(Secret::new(token.clone()));
        }
        let client = builder.build()?;

        // Scope probes: /repos/explore needs `repo` (the middleware 403s a
        // PAT without it, while anonymous browse is allowed — the PAT
        // presence is what triggers enforcement), /users/me needs `user`.
        let repo_scope = probe(
            client
                .raw(ifg_client::Method::GET, "repos/explore?per_page=1", None)
                .await,
        );
        let user_probe = client.users().me().await;
        let user_scope = match &user_probe {
            Ok(_) => Scope::Ok,
            Err(err) if err.is_unauthorized() || err.is_forbidden() => Scope::Denied,
            Err(_) => Scope::Unreachable,
        };

        let live_username = user_probe.ok().map(|me| me.username);
        let username = live_username
            .or_else(|| entry.user.clone())
            .unwrap_or_else(|| "?".into());
        let token_display = entry
            .token
            .as_ref()
            .map(|token| Secret::new(token.clone()).redacted())
            .unwrap_or_else(|| "—".into());

        let server = super::version::server_label(&client).await;
        rows.push(vec![
            name.clone(),
            entry.url.clone(),
            username.clone(),
            token_display.clone(),
            repo_scope.glyph().to_string(),
            user_scope.glyph().to_string(),
            server.clone(),
        ]);
        json_rows.push(json!({
            "host": name,
            "url": entry.url,
            "user": username,
            "token": if entry.token.is_some() { Some(token_display) } else { None },
            "repo_scope": repo_scope.label(),
            "user_scope": user_scope.label(),
            "server_version": server,
        }));
    }

    if cli.json {
        output::print_json(&serde_json::Value::Array(json_rows));
    } else {
        output::print_table(
            &[
                "HOST",
                "URL",
                "USER",
                "TOKEN",
                "REPO",
                "USER-SCOPE",
                "SERVER",
            ],
            &rows,
        );
        eprintln!("(✓ scope works, ✗ scope denied, ? host unreachable)");
    }
    Ok(())
}

fn probe(result: Result<Vec<u8>, ifg_client::Error>) -> Scope {
    match result {
        Ok(_) => Scope::Ok,
        Err(err) if err.is_unauthorized() || err.is_forbidden() => Scope::Denied,
        Err(_) => Scope::Unreachable,
    }
}

fn logout(cli: &Cli) -> Result<(), AppError> {
    let mut config = config::load()?;
    let target = logout_host(&cli.host, &config)?;

    config.hosts.remove(&target);
    if config.default_host.as_deref() == Some(target.as_str()) {
        config.default_host = config.hosts.keys().next().cloned();
    }
    config::save(&config)?;
    if let Err(error) = crate::keyring::delete(&target) {
        eprintln!("ifg: warning: could not remove the keychain entry: {error}");
    }
    println!(
        "Removed credentials for {target}. The token remains valid server-side — revoke it in \
         the web UI (Settings → Tokens)."
    );
    Ok(())
}

fn logout_host(host_arg: &Option<String>, config: &Config) -> Result<String, AppError> {
    let names: Vec<&String> = config.hosts.keys().collect();
    if let Some(host_arg) = host_arg {
        if context::is_url(host_arg) {
            let key = config::host_key_for_url(host_arg)
                .ok_or_else(|| AppError::Other(format!("invalid host URL: {host_arg}")))?;
            if config.hosts.contains_key(&key) {
                return Ok(key);
            }
        } else if config.hosts.contains_key(host_arg) {
            return Ok(host_arg.clone());
        }
        return Err(AppError::Other(format!(
            "host '{host_arg}' is not configured; configured: {}",
            names
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    match names.len() {
        0 => Err(AppError::Other("no hosts configured".into())),
        1 => Ok(names[0].clone()),
        _ => {
            if let Some(default) = &config.default_host {
                if config.hosts.contains_key(default) {
                    return Ok(default.clone());
                }
            }
            Err(AppError::Usage(format!(
                "several hosts configured; pass --host. Configured: {}",
                names
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )))
        }
    }
}

fn token(cli: &Cli) -> Result<(), AppError> {
    let config = config::load()?;
    let resolved = crate::context::resolve_host(cli.host.as_deref(), cli.repo.as_deref(), &config)?;
    let host_label = resolved.name.as_deref().unwrap_or(resolved.url.as_str());
    match resolved.token {
        Some(secret) => {
            println!("{}", secret.expose());
            Ok(())
        }
        None => Err(AppError::Auth(format!(
            "no token for {host_label}; run `ifg auth login`"
        ))),
    }
}

/// git credential helper protocol (design doc §4.1, §5.3): answer `get`
/// requests when — and only when — both the protocol and the host match a
/// configured IronForge host exactly; anything else stays silent so the PAT
/// can never leak to another site, nor go out in cleartext over `http` for a
/// host configured as `https`. `store`/`erase` are no-ops.
fn git_credential(action: &str) -> Result<(), AppError> {
    use std::io::Read;

    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| AppError::Other(format!("could not read credential request: {e}")))?;
    if action != "get" {
        return Ok(());
    }
    let config = config::load()?;
    if let Some(answer) = credential_answer(&config, &input) {
        print!("{answer}");
        use std::io::Write;
        let _ = std::io::stdout().flush();
    }
    Ok(())
}

/// The helper's reply to a `get` request, or `None` to stay silent (git
/// then moves on to the next helper).
fn credential_answer(config: &Config, request: &str) -> Option<String> {
    let mut protocol: Option<&str> = None;
    let mut host: Option<&str> = None;
    for line in request.lines() {
        match line.split_once('=') {
            Some(("protocol", value)) => protocol = Some(value),
            Some(("host", value)) => host = Some(value),
            _ => {}
        }
    }
    let (protocol, host) = (protocol?, host?);

    let entry = config.hosts.values().find(|entry| {
        url::Url::parse(&entry.url).is_ok_and(|url| url.scheme() == protocol)
            && config::host_key_for_url(&entry.url).as_deref() == Some(host)
    })?;
    let token = entry.token.as_deref()?;
    let username = entry.user.as_deref().unwrap_or("ifg");
    Some(format!(
        "protocol={protocol}\nhost={host}\nusername={username}\npassword={token}\n\n"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with(url: &str) -> Config {
        let mut config = Config::default();
        let mut entry = HostEntry::new(url);
        entry.user = Some("alice".into());
        entry.token = Some("ifp_secret".into());
        config
            .hosts
            .insert(config::host_key_for_url(url).unwrap(), entry);
        config
    }

    #[test]
    fn answers_matching_protocol_and_host() {
        let config = config_with("https://forge.example.com");
        let answer =
            credential_answer(&config, "protocol=https\nhost=forge.example.com\n").unwrap();
        assert!(answer.contains("username=alice\n"));
        assert!(answer.contains("password=ifp_secret\n"));
    }

    #[test]
    fn https_token_is_never_offered_over_http() {
        let config = config_with("https://forge.example.com");
        assert!(credential_answer(&config, "protocol=http\nhost=forge.example.com\n").is_none());
    }

    #[test]
    fn other_hosts_ports_and_incomplete_requests_get_nothing() {
        let config = config_with("https://forge.example.com");
        assert!(credential_answer(&config, "protocol=https\nhost=evil.example.com\n").is_none());
        assert!(
            credential_answer(&config, "protocol=https\nhost=forge.example.com:8443\n").is_none()
        );
        assert!(credential_answer(&config, "host=forge.example.com\n").is_none());
        assert!(credential_answer(&config, "protocol=https\n").is_none());
    }

    #[test]
    fn plain_http_hosts_still_work_when_configured_as_http() {
        let config = config_with("http://localhost:8080");
        let answer = credential_answer(&config, "protocol=http\nhost=localhost:8080\n").unwrap();
        assert!(answer.contains("password=ifp_secret\n"));
    }
}
