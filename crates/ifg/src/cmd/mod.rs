//! Subcommand implementations (M1: `auth`, `api`; M2: `repo`, `issue`, `pr`;
//! M3: `ci`, `release`; M5: `label`, `milestone`, `status`).

pub mod api;
pub mod auth;
pub mod ci;
pub mod collab;
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
pub mod wiki;

use ifg_client::Client;

use crate::cli::Cli;
use crate::config::Config;
use crate::context::{self, ResolvedHost};
use crate::error::AppError;

/// Commands that produce documents, invoke git, or open a browser cannot
/// silently accept structured-output flags. Validate before any side effect.
pub(crate) fn validate_output(cli: &Cli) -> Result<(), AppError> {
    use crate::cli::{
        AuthCommand, CiCommand, Command, IssueCommand, LabelCommand, MilestoneCommand, PrCommand,
        ReleaseCommand, RepoCommand,
    };
    if !cli.json && cli.template.is_none() {
        return Ok(());
    }
    let supported = match &cli.command {
        Command::Api { .. }
        | Command::Status
        | Command::Search { .. }
        | Command::Doctor
        | Command::Collab { .. }
        | Command::Wiki { .. }
        | Command::Org { .. }
        | Command::Notifications { .. } => true,
        Command::Auth { command } => matches!(command, AuthCommand::Status),
        Command::Repo { command } => matches!(
            command,
            RepoCommand::Collaborators { .. }
                | RepoCommand::List { .. }
                | RepoCommand::View { web: false, .. }
                | RepoCommand::Edit { .. }
                | RepoCommand::Create { .. }
        ),
        Command::Issue { command } => matches!(
            command,
            IssueCommand::List { .. }
                | IssueCommand::View { web: false, .. }
                | IssueCommand::Create { .. }
                | IssueCommand::Comment { .. }
                | IssueCommand::Close { .. }
                | IssueCommand::Reopen { .. }
                | IssueCommand::Edit { .. }
        ),
        Command::Pr { command } => matches!(
            command,
            PrCommand::List { .. }
                | PrCommand::View { web: false, .. }
                | PrCommand::Create { .. }
                | PrCommand::Merge { .. }
                | PrCommand::Review { .. }
                | PrCommand::Edit { .. }
                | PrCommand::Close { .. }
                | PrCommand::Reopen { .. }
                | PrCommand::Comment { .. }
                | PrCommand::Reviews { .. }
                | PrCommand::Reviewers { .. }
        ),
        Command::Ci { command } => matches!(
            command,
            CiCommand::Run { .. }
                | CiCommand::Wait { .. }
                | CiCommand::Runs { .. }
                | CiCommand::View { .. }
                | CiCommand::Cancel { .. }
                | CiCommand::Retry { .. }
                | CiCommand::Approve { .. }
        ),
        Command::Release { command } => matches!(
            command,
            ReleaseCommand::List { .. }
                | ReleaseCommand::View { web: false, .. }
                | ReleaseCommand::Create { .. }
                | ReleaseCommand::Edit { .. }
        ),
        Command::Label { command } => matches!(
            command,
            LabelCommand::List | LabelCommand::Create { .. } | LabelCommand::Edit { .. }
        ),
        Command::Milestone { command } => matches!(
            command,
            MilestoneCommand::List
                | MilestoneCommand::Create { .. }
                | MilestoneCommand::Edit { .. }
        ),
        _ => false,
    };
    if supported {
        Ok(())
    } else {
        Err(AppError::Usage(
            "this command does not support --json or --template".into(),
        ))
    }
}

pub(crate) fn show(cli: &Cli, value: &serde_json::Value) {
    match &cli.template {
        Some(template) => match value.as_array() {
            Some(items) => crate::output::print_template(items, template),
            None => crate::output::print_template(std::slice::from_ref(value), template),
        },
        None => crate::output::print_json(value),
    }
}

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
    let host = context::resolve_host(
        cli.host.as_deref(),
        cli.repo.as_deref(),
        cli.target_hint.as_deref(),
        &config,
    )?;
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
    let parts = parse_editor(editor)?;
    let file = tempfile::Builder::new()
        .prefix("ifg-edit-")
        .suffix(".md")
        .tempfile()
        .map_err(|e| AppError::Io(format!("could not create editor file: {e}")))?;
    let path = file.path();
    let template = format!(
        "{initial_title}\n\n{initial_body}\n\n# First line above is the title; the rest is the body.\n# Lines starting with '#' are dropped. Save an empty file to abort.\n"
    );
    std::fs::write(path, template)
        .map_err(|e| AppError::Other(format!("could not write {}: {e}", path.display())))?;
    let status = std::process::Command::new(&parts[0])
        .args(&parts[1..])
        .arg(path)
        .status()
        .map_err(|e| AppError::Other(format!("could not run editor {}: {e}", parts[0])))?;
    if !status.success() {
        return Err(AppError::Other(format!(
            "editor {} exited with {status}",
            parts[0]
        )));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| AppError::Other(format!("could not read {}: {e}", path.display())))?;

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

/// Split an editor command into an executable and arguments without invoking a
/// shell. Quotes group spaces; backslashes in Windows paths are preserved.
fn parse_editor(input: &str) -> Result<Vec<String>, AppError> {
    // Preserve the previous ability to pass a bare executable path with spaces.
    if std::path::Path::new(input).is_file() {
        return Ok(vec![input.to_owned()]);
    }
    let mut parts = Vec::new();
    let mut token = String::new();
    let mut chars = input.chars().peekable();
    let mut quote = None;
    let mut started = false;
    while let Some(ch) = chars.next() {
        match (quote, ch) {
            (None, '\'' | '"') => {
                quote = Some(ch);
                started = true;
            }
            (Some(q), ch) if q == ch => quote = None,
            (None, ch) if ch.is_whitespace() => {
                if started {
                    parts.push(std::mem::take(&mut token));
                    started = false;
                }
            }
            (None, '\\')
                if chars
                    .peek()
                    .is_some_and(|c| c.is_whitespace() || *c == '\'' || *c == '"') =>
            {
                token.push(chars.next().expect("peeked"));
                started = true;
            }
            (Some('"'), '\\') if chars.peek() == Some(&'"') => {
                token.push(chars.next().expect("peeked"));
                started = true;
            }
            (_, ch) => {
                token.push(ch);
                started = true;
            }
        }
    }
    if quote.is_some() {
        return Err(AppError::Usage("unclosed quote in editor command".into()));
    }
    if started {
        parts.push(token);
    }
    if parts.first().is_none_or(String::is_empty) {
        return Err(AppError::Usage("editor command is empty".into()));
    }
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_arguments_handle_quotes_escapes_and_windows_paths() {
        assert_eq!(parse_editor("code --wait").unwrap(), ["code", "--wait"]);
        assert_eq!(
            parse_editor(r#""C:\Program Files\Editor\edit.exe" --wait "two words""#).unwrap(),
            [r"C:\Program Files\Editor\edit.exe", "--wait", "two words"]
        );
        assert_eq!(
            parse_editor(r"editor\ with\ spaces --wait").unwrap(),
            ["editor with spaces", "--wait"]
        );
        assert_eq!(
            parse_editor("editor '$HOME; echo danger'").unwrap(),
            ["editor", "$HOME; echo danger"]
        );
        assert!(parse_editor("editor 'broken").is_err());
        assert!(parse_editor("   ").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn editor_executes_quoted_path_with_arguments_and_cleans_up() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::Builder::new()
            .prefix("ifg editor ")
            .tempdir()
            .unwrap();
        let script = dir.path().join("fake editor.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\n[ \"$1\" = --wait ] || exit 3\nprintf 'Title\\n\\nBody\\n' > \"$2\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let editor = format!("\"{}\" --wait", script.display());
        assert_eq!(
            compose_with_editor(&editor, "", "").unwrap(),
            ("Title".into(), "Body".into())
        );
        assert_eq!(
            parse_editor(script.to_str().unwrap()).unwrap(),
            [script.to_str().unwrap()]
        );
        let record = dir.path().join("record");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s' \"$1\" > '{}'\nexit 1\n",
                record.display()
            ),
        )
        .unwrap();
        assert!(compose_with_editor(script.to_str().unwrap(), "", "").is_err());
        let path = std::fs::read_to_string(record).unwrap();
        assert!(
            !std::path::Path::new(&path).exists(),
            "temporary file must be removed after failure"
        );
    }

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

    /// Same flow with a batch-file fake editor: `Command` runs `.cmd` through
    /// `cmd.exe` on Windows, mirroring how a real editor path is spawned.
    #[cfg(windows)]
    #[test]
    fn editor_composition_splits_title_and_body_windows() {
        let dir = std::env::temp_dir().join(format!("ifg-editor-test-win-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let script = dir.join("fake-editor.cmd");
        std::fs::write(
            &script,
            "@echo off\r\n\
             >\"%~1\" echo Fix the parser\r\n\
             >>\"%~1\" echo.\r\n\
             >>\"%~1\" echo Longer explanation.\r\n\
             >>\"%~1\" echo # a comment line\r\n",
        )
        .expect("script");

        let result = compose_with_editor(script.to_str().expect("utf8 path"), "", "");
        let _ = std::fs::remove_dir_all(&dir);

        let (title, body) = result.expect("composed");
        assert_eq!(title, "Fix the parser");
        assert_eq!(body, "Longer explanation.");
    }
}
