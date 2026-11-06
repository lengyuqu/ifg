//! Agent collaboration commands. Session credentials are scoped to one host
//! and repository, and are never printed or accepted as command-line flags.
use std::path::Path;

use ifg_client::{collab_segment, Method, Secret, SessionCredentials};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::cli::{
    Cli, CollabCommand, CollabHandoffCommand, CollabMessageCommand, CollabSessionCommand,
    CollabTaskCommand,
};
use crate::context::resolve_repo;
use crate::error::AppError;
use crate::output;

#[derive(Serialize, Deserialize)]
struct StoredSession {
    host: String,
    repo: String,
    id: String,
    secret: String,
}

fn session_path(path: Option<&Path>) -> Result<&Path, AppError> {
    path.ok_or_else(|| AppError::Usage("this operation requires --session-file".into()))
}

#[cfg(unix)]
fn create_private(path: &Path) -> Result<std::fs::File, AppError> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| {
            AppError::Io(format!(
                "cannot create session file {}: {e}",
                path.display()
            ))
        })
}
#[cfg(not(unix))]
fn create_private(_path: &Path) -> Result<std::fs::File, AppError> {
    Err(AppError::Other(
        "secure session file storage is not yet supported on this platform".into(),
    ))
}

fn load_session(path: &Path, host: &str, repo: &str) -> Result<SessionCredentials, AppError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|e| AppError::Io(format!("cannot inspect session file: {e}")))?;
    if !metadata.file_type().is_file() {
        return Err(AppError::Auth(
            "session file must be a regular file, not a symlink".into(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(AppError::Auth(
                "session file must not be accessible by group or others (chmod 600)".into(),
            ));
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (host, repo);
        return Err(AppError::Other(
            "secure session file storage is not yet supported on this platform".into(),
        ));
    }
    #[cfg(unix)]
    {
        use std::io::Read;
        use std::os::unix::fs::MetadataExt;
        let file = std::fs::File::open(path)
            .map_err(|e| AppError::Io(format!("cannot open session file: {e}")))?;
        let opened = file
            .metadata()
            .map_err(|e| AppError::Io(format!("cannot inspect session file: {e}")))?;
        if !opened.is_file()
            || opened.dev() != metadata.dev()
            || opened.ino() != metadata.ino()
            || opened.mode() & 0o077 != 0
            || opened.nlink() != 1
        {
            return Err(AppError::Auth(
                "session file changed or is not private".into(),
            ));
        }
        let mut bytes = Vec::new();
        file.take(16 * 1024)
            .read_to_end(&mut bytes)
            .map_err(|e| AppError::Io(format!("cannot read session file: {e}")))?;
        let stored: StoredSession = serde_json::from_slice(&bytes)
            .map_err(|_| AppError::Auth("invalid session file".into()))?;
        if stored.host != host
            || stored.repo != repo
            || stored.id.is_empty()
            || stored.secret.is_empty()
        {
            return Err(AppError::Auth(
                "session file does not match this host and repository".into(),
            ));
        }
        Ok(SessionCredentials::new(
            stored.id,
            Secret::new(stored.secret),
        ))
    }
}

fn query(path: &str, params: &[(&str, Option<String>)]) -> String {
    let mut encoded = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in params {
        if let Some(value) = value {
            encoded.append_pair(key, value);
        }
    }
    let q = encoded.finish();
    if q.is_empty() {
        path.to_string()
    } else {
        format!("{path}?{q}")
    }
}

fn show(cli: &Cli, value: &Value) {
    if let Some(template) = &cli.template {
        if let Some(items) = value.get("items").and_then(Value::as_array) {
            output::print_template(items, template);
        } else {
            output::print_template(std::slice::from_ref(value), template);
        }
    } else {
        output::print_json(value);
    }
}

pub async fn run(command: &CollabCommand, file: Option<&Path>, cli: &Cli) -> Result<(), AppError> {
    let ctx = super::bootstrap(cli)?;
    let repo = resolve_repo(cli.repo.as_deref(), &ctx.host)?;
    let repo_key = repo.to_string();
    let collab = ctx.client.collaboration(&repo.owner, &repo.name);

    if let CollabCommand::Session {
        command: CollabSessionCommand::Start { name, client },
    } = command
    {
        let path = session_path(file)?;
        let mut output_file = create_private(path)?;
        let result = async {
            let started = collab.start_session(name, client).await?;
            let record = StoredSession {
                host: ctx.host.url.clone(),
                repo: repo_key.clone(),
                id: started.credentials.id.clone(),
                secret: started.credentials.secret().expose().to_owned(),
            };
            serde_json::to_writer(&mut output_file, &record)
                .map_err(|_| AppError::Io("could not serialize session file".into()))?;
            output_file.sync_all().map_err(|_| {
                AppError::Io(
                    "could not persist session credentials; created session cannot be resumed"
                        .into(),
                )
            })?;
            Ok::<Value, AppError>(started.session)
        }
        .await;
        match result {
            Ok(session) => {
                show(cli, &session);
                return Ok(());
            }
            Err(err) => {
                drop(output_file);
                let _ = std::fs::remove_file(path);
                return Err(err);
            }
        }
    }

    let need_session = matches!(
        command,
        CollabCommand::Session {
            command: CollabSessionCommand::Heartbeat | CollabSessionCommand::End
        } | CollabCommand::Task {
            command: CollabTaskCommand::Create { .. } | CollabTaskCommand::Update { .. }
        } | CollabCommand::Message {
            command: CollabMessageCommand::Send { .. } | CollabMessageCommand::Read { .. }
        } | CollabCommand::Handoff {
            command: CollabHandoffCommand::Create { .. }
                | CollabHandoffCommand::Accept { .. }
                | CollabHandoffCommand::Cancel { .. }
        } | CollabCommand::Inbox { .. }
    );
    let session = if need_session {
        Some(load_session(session_path(file)?, &ctx.host.url, &repo_key)?)
    } else {
        None
    };
    let encoded = |s: &str| collab_segment(s);
    let (method, path, body) = match command {
        CollabCommand::Session { command } => match command {
            CollabSessionCommand::Start { .. } => unreachable!(),
            CollabSessionCommand::List { cursor, limit } => (
                Method::GET,
                query(
                    "sessions",
                    &[
                        ("cursor", cursor.clone()),
                        ("limit", limit.map(|n| n.to_string())),
                    ],
                ),
                None,
            ),
            CollabSessionCommand::Heartbeat => (
                Method::POST,
                format!(
                    "sessions/{}/heartbeat",
                    encoded(&session.as_ref().unwrap().id)
                ),
                None,
            ),
            CollabSessionCommand::End => (
                Method::POST,
                format!("sessions/{}/end", encoded(&session.as_ref().unwrap().id)),
                None,
            ),
        },
        CollabCommand::Task { command } => match command {
            CollabTaskCommand::List {
                status,
                cursor,
                limit,
            } => (
                Method::GET,
                query(
                    "tasks",
                    &[
                        ("status", status.clone()),
                        ("cursor", cursor.clone()),
                        ("limit", limit.map(|n| n.to_string())),
                    ],
                ),
                None,
            ),
            CollabTaskCommand::View { id } => (Method::GET, format!("tasks/{}", encoded(id)), None),
            CollabTaskCommand::Create {
                title,
                description,
                owner_session,
                branch,
                path,
                issue,
                pr,
            } => (
                Method::POST,
                "tasks".into(),
                Some(
                    json!({"title":title,"description":description,"owner_session_id":owner_session,"branch":branch,"paths":path,"issue_number":issue,"pr_number":pr}),
                ),
            ),
            CollabTaskCommand::Update {
                id,
                expected_version,
                status,
                title,
                description,
                clear_description,
                owner_session,
                clear_owner_session,
                branch,
                clear_branch,
                path,
                clear_paths,
                issue,
                clear_issue,
                pr,
                clear_pr,
            } => {
                let mut body = json!({"expected_version":expected_version});
                for (key, value) in [
                    ("status", status),
                    ("title", title),
                    ("description", description),
                    ("owner_session_id", owner_session),
                    ("branch", branch),
                ] {
                    if let Some(value) = value {
                        body[key] = json!(value);
                    }
                }
                for (key, clear) in [
                    ("description", clear_description),
                    ("owner_session_id", clear_owner_session),
                    ("branch", clear_branch),
                    ("issue_number", clear_issue),
                    ("pr_number", clear_pr),
                ] {
                    if *clear {
                        body[key] = Value::Null;
                    }
                }
                if !path.is_empty() {
                    body["paths"] = json!(path);
                }
                if *clear_paths {
                    body["paths"] = json!([]);
                }
                if let Some(issue) = issue {
                    body["issue_number"] = json!(issue);
                }
                if let Some(pr) = pr {
                    body["pr_number"] = json!(pr);
                }
                (Method::PATCH, format!("tasks/{}", encoded(id)), Some(body))
            }
        },
        CollabCommand::Message { command } => match command {
            CollabMessageCommand::List {
                task,
                after_seq,
                limit,
            } => (
                Method::GET,
                query(
                    &format!("tasks/{}/messages", encoded(task)),
                    &[
                        ("after_seq", Some(after_seq.to_string())),
                        ("limit", limit.map(|n| n.to_string())),
                    ],
                ),
                None,
            ),
            CollabMessageCommand::Send {
                task,
                key,
                kind,
                body,
                reply_to,
                target_session,
                commit,
            } => (
                Method::POST,
                format!("tasks/{}/messages", encoded(task)),
                Some(
                    json!({"client_message_id":key,"kind":kind,"body":body,"reply_to_id":reply_to,"target_session_id":target_session,"commit_sha":commit}),
                ),
            ),
            CollabMessageCommand::Read { task, seq } => (
                Method::PUT,
                format!("tasks/{}/read", encoded(task)),
                Some(json!({"last_read_seq":seq})),
            ),
        },
        CollabCommand::Handoff { command } => match command {
            CollabHandoffCommand::List {
                task,
                cursor,
                limit,
            } => (
                Method::GET,
                query(
                    &format!("tasks/{}/handoffs", encoded(task)),
                    &[
                        ("cursor", cursor.clone()),
                        ("limit", limit.map(|n| n.to_string())),
                    ],
                ),
                None,
            ),
            CollabHandoffCommand::Create {
                task,
                key,
                to,
                summary,
                completed,
                next_steps,
                blockers,
                commit,
                evidence,
            } => (
                Method::POST,
                format!("tasks/{}/handoffs", encoded(task)),
                Some(
                    json!({"client_handoff_id":key,"to_session_id":to,"summary":summary,"completed":completed,"next_steps":next_steps,"blockers":blockers,"commit_sha":commit,"evidence":evidence}),
                ),
            ),
            CollabHandoffCommand::Accept {
                id,
                expected_version,
            } => (
                Method::POST,
                format!("handoffs/{}/accept", encoded(id)),
                Some(json!({"expected_version":expected_version})),
            ),
            CollabHandoffCommand::Cancel {
                id,
                expected_version,
            } => (
                Method::POST,
                format!("handoffs/{}/cancel", encoded(id)),
                Some(json!({"expected_version":expected_version})),
            ),
        },
        CollabCommand::Inbox { since, limit } => (
            Method::GET,
            query(
                "inbox",
                &[
                    ("since", since.clone()),
                    ("limit", limit.map(|n| n.to_string())),
                ],
            ),
            None,
        ),
    };
    let value = collab
        .request(method, &path, body.as_ref(), session.as_ref())
        .await?;
    show(cli, &value);
    Ok(())
}
