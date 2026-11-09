//! `ifg version` and the server-version probe (design doc D10).

use crate::cli::Cli;
use crate::cmd::bootstrap;
use crate::error::AppError;
use crate::output;

pub async fn run(cli: &Cli) -> Result<(), AppError> {
    output::line!("ifg {}", env!("CARGO_PKG_VERSION"));

    // Version must work without any configuration: resolve the host
    // best-effort and report a missing one instead of failing.
    let ctx = match bootstrap(cli) {
        Ok(ctx) => ctx,
        Err(error) => {
            output::line!("server: not reachable ({error})");
            return Ok(());
        }
    };
    match ctx.client.health().await {
        Ok(health) => {
            let version = health["version"].as_str().unwrap_or("unknown");
            let mut line = format!("server: {version}");
            if let Some(phase) = health["phase"].as_u64() {
                line.push_str(&format!(" (phase {phase})"));
            }
            output::line!("{line}");
            output::line!("host:   {}", ctx.host.url);
            // The instance endpoint can require a broader PAT scope. When a
            // repository is known, its scoped discovery also works with a
            // repository-limited token.
            let mut discovery = ctx.client.capabilities().await;
            if discovery.is_err() {
                if let Ok(repo) = crate::context::resolve_repo(cli.repo.as_deref(), &ctx.host) {
                    discovery = ctx
                        .client
                        .send(
                            ifg_client::Method::GET,
                            &format!(
                                "repos/{}/{}/collaboration/capabilities",
                                repo.owner, repo.name
                            ),
                            None,
                        )
                        .await;
                }
            }
            if let Ok(capabilities) = discovery {
                if let Some(commit) = capabilities["build_commit"].as_str() {
                    let dirty = match capabilities["build_dirty"].as_bool() {
                        Some(true) => " (modified source)",
                        Some(false) => "",
                        None => " (source state unknown)",
                    };
                    output::line!("build:  {commit}{dirty}");
                } else {
                    output::line!("build:  unknown");
                }
                if let Some(features) = capabilities["api_capabilities"].as_array() {
                    let features: Vec<_> = features.iter().filter_map(|v| v.as_str()).collect();
                    output::line!("api capabilities: {}", features.join(", "));
                } else {
                    output::line!("api capabilities: unknown");
                }
            } else {
                output::line!("build:  unknown (capability discovery unavailable)");
                output::line!("api capabilities: unknown");
            }
            if let Some(message) = crate::version::outdated(version) {
                eprintln!("ifg: warning: {message}");
            }
        }
        Err(error) => output::line!("server: not reachable ({error})"),
    }
    Ok(())
}

/// One cell for `auth status`: the server version, or `?` when unreachable.
pub(crate) async fn server_label(client: &ifg_client::Client) -> String {
    match client.health().await {
        Ok(health) => health["version"].as_str().unwrap_or("?").to_string(),
        Err(_) => "?".to_string(),
    }
}
