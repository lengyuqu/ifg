//! Subcommand implementations (M1: `auth`, `api`; M2: `repo`, `issue`, `pr`;
//! M3: `ci`, `release`; M5: `label`, `milestone`, `status`).

pub mod api;
pub mod auth;
pub mod ci;
pub mod doctor;
pub mod issue;
pub mod label;
pub mod milestone;
pub mod notifications;
pub mod org;
pub mod pr;
pub mod release;
pub mod repo;
pub mod search;
pub mod status;
pub mod version;

use ifg_client::Client;

use crate::cli::Cli;
use crate::config::Config;
use crate::context::{self, ResolvedHost};
use crate::error::AppError;

/// Everything a command needs after flag parsing: the config, the resolved
/// host and a client bound to it.
pub(crate) struct CommandContext {
    pub config: Config,
    pub host: ResolvedHost,
    pub client: Client,
}

pub(crate) fn user_agent() -> String {
    concat!("ifg/", env!("CARGO_PKG_VERSION")).to_string()
}

/// Load the config, resolve the host (§5.2) and build the client.
pub(crate) fn bootstrap(cli: &Cli) -> Result<CommandContext, AppError> {
    let config = crate::config::load()?;
    let host = context::resolve_host(cli.host.as_deref(), cli.repo.as_deref(), &config)?;
    let mut builder = Client::builder(&host.url).user_agent(user_agent());
    if let Some(token) = &host.token {
        builder = builder.token(token.clone());
    }
    let client = builder.build()?;
    Ok(CommandContext {
        config,
        host,
        client,
    })
}

/// Yes/no prompt on a terminal; `--yes` skips it and non-interactive runs
/// refuse rather than assume.
pub(crate) fn confirm(question: &str) -> Result<bool, AppError> {
    use std::io::{IsTerminal, Write};
    if !std::io::stdin().is_terminal() {
        return Err(AppError::Usage(format!(
            "{question} — rerun with --yes to confirm non-interactively"
        )));
    }
    eprint!("{question} [y/N] ");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| AppError::Other(format!("could not read the answer: {e}")))?;
    Ok(matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

/// Compose a title and body in `$EDITOR` (design doc §5.4). The first
/// non-empty, non-comment line is the title; everything after it is the body.
/// Lines starting with `#` are instructions and are dropped.
pub(crate) fn compose_in_editor(
    initial_title: &str,
    initial_body: &str,
) -> Result<(String, String), AppError> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .map_err(|_| AppError::Usage("--editor needs $EDITOR (or $VISUAL) to be set".into()))?;
    compose_with_editor(&editor, initial_title, initial_body)
}

/// The editor invocation without the environment lookup, so it can be tested
/// with a fake editor script.
fn compose_with_editor(
    editor: &str,
    initial_title: &str,
    initial_body: &str,
) -> Result<(String, String), AppError> {
    let path = std::env::temp_dir().join(format!("ifg-edit-{}.md", std::process::id()));
    let template = format!(
        "{initial_title}\n\n{initial_body}\n\n# First line above is the title; the rest is the body.\n# Lines starting with '#' are dropped. Save an empty file to abort.\n"
    );
    std::fs::write(&path, template)
        .map_err(|e| AppError::Other(format!("could not write {}: {e}", path.display())))?;
    let status = std::process::Command::new(editor)
        .arg(&path)
        .status()
        .map_err(|e| AppError::Other(format!("could not run {editor}: {e}")))?;
    if !status.success() {
        let _ = std::fs::remove_file(&path);
        return Err(AppError::Other(format!("{editor} exited with {status}")));
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| AppError::Other(format!("could not read {}: {e}", path.display())))?;
    let _ = std::fs::remove_file(&path);

    let mut lines = text
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .skip_while(|line| line.trim().is_empty());
    let title = lines.next().unwrap_or("").trim().to_string();
    let body = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    if title.is_empty() {
        return Err(AppError::Usage("empty title; nothing created".into()));
    }
    Ok((title, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `--editor`: first non-comment line is the title, the rest the body.
    #[cfg(unix)]
    #[test]
    fn editor_composition_splits_title_and_body() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("ifg-editor-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let script = dir.join("fake-editor.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\nprintf 'Fix the parser\\n\\nLonger explanation.\\n# a comment line\\n' > \"$1\"\n",
        )
        .expect("script");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");

        let result = compose_with_editor(script.to_str().expect("utf8 path"), "", "");
        let _ = std::fs::remove_dir_all(&dir);

        let (title, body) = result.expect("composed");
        assert_eq!(title, "Fix the parser");
        assert_eq!(body, "Longer explanation.");
    }
}
