//! Agent collaboration commands. Session credentials are scoped to one host
//! and repository, and are never printed or accepted as command-line flags.
use std::path::Path;

#[cfg(unix)]
use ifg_client::Secret;
use ifg_client::{collab_segment, Method, SessionCredentials};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::cli::{
    Cli, CollabBudgetCommand, CollabCommand, CollabConditionCommand, CollabDecisionCommand,
    CollabDependencyCommand, CollabHandoffCommand, CollabMessageCommand, CollabQuestionCommand,
    CollabSessionCommand, CollabTaskCommand, CollabUsageCommand, CollabWorkPackageCommand,
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
        Err(AppError::Other(
            "secure session file storage is not yet supported on this platform".into(),
        ))
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

/// Read the body for `condition set`. The file is sent as-is, so the only
/// check is that it parses as a JSON object; anything else is a usage error
/// (exit 2) raised before any request.
fn read_conditions_body(source: &str) -> Result<Value, AppError> {
    let text = if source == "-" {
        use std::io::Read;
        let mut buffer = String::new();
        std::io::stdin()
            .read_to_string(&mut buffer)
            .map_err(|e| AppError::Io(format!("cannot read stdin: {e}")))?;
        buffer
    } else {
        std::fs::read_to_string(source)
            .map_err(|e| AppError::Io(format!("cannot read {source}: {e}")))?
    };
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| AppError::Usage(format!("condition file is not valid JSON: {e}")))?;
    if !value.is_object() {
        return Err(AppError::Usage(
            "condition file must contain a JSON object".into(),
        ));
    }
    Ok(value)
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
            command: CollabTaskCommand::Create { .. }
                | CollabTaskCommand::Update { .. }
                | CollabTaskCommand::Claim { .. }
        } | CollabCommand::Message {
            command: CollabMessageCommand::Send { .. } | CollabMessageCommand::Read { .. }
        } | CollabCommand::Handoff {
            command: CollabHandoffCommand::Create { .. }
                | CollabHandoffCommand::Accept { .. }
                | CollabHandoffCommand::Cancel { .. }
        } | CollabCommand::WorkPackage {
            command: CollabWorkPackageCommand::Create { .. }
        } | CollabCommand::Dependency {
            command: CollabDependencyCommand::Add { .. } | CollabDependencyCommand::Remove { .. }
        } | CollabCommand::Decision {
            command: CollabDecisionCommand::Create { .. }
        } | CollabCommand::Condition {
            command: CollabConditionCommand::Set { .. } | CollabConditionCommand::Confirm { .. }
        } | CollabCommand::Usage {
            command: CollabUsageCommand::Report { .. }
        } | CollabCommand::Inbox { user: false, .. }
    );
    let optional_session = matches!(
        command,
        CollabCommand::Question {
            command: CollabQuestionCommand::Ask { .. }
                | CollabQuestionCommand::Answer { .. }
                | CollabQuestionCommand::Cancel { .. }
        }
    );
    if file.is_some()
        && matches!(
            command,
            CollabCommand::Inbox { user: true, .. }
                | CollabCommand::Task {
                    command: CollabTaskCommand::Reassign { .. }
                }
        )
    {
        return Err(AppError::Usage(
            "this operation acts as the account user; omit --session-file".into(),
        ));
    }
    let session = if need_session || (optional_session && file.is_some()) {
        Some(load_session(session_path(file)?, &ctx.host.url, &repo_key)?)
    } else {
        None
    };
    // `need_session` and `optional_session` decide whether a session file is
    // loaded; the routing below reads `session_id`. A drift between the two
    // lists used to panic here, so fail with a usage error instead.
    if need_session && session.is_none() {
        return Err(AppError::Usage(
            "this operation requires --session-file".into(),
        ));
    }
    let encoded = |s: &str| collab_segment(s);
    let session_id = session
        .as_ref()
        .map(|credentials| encoded(&credentials.id))
        .unwrap_or_default();
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
                format!("sessions/{session_id}/heartbeat"),
                None,
            ),
            CollabSessionCommand::End => (Method::POST, format!("sessions/{session_id}/end"), None),
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
            CollabTaskCommand::Status { id } => (
                Method::GET,
                format!("tasks/{}/completion-status", encoded(id)),
                None,
            ),
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
            CollabTaskCommand::Claim {
                id,
                key,
                expected_version,
                expected_owner,
                reason,
            } => (
                Method::POST,
                format!("tasks/{}/takeovers", encoded(id)),
                Some(
                    json!({"client_takeover_id":key,"expected_task_version":expected_version,"expected_owner_session_id":expected_owner,"reason":reason}),
                ),
            ),
            CollabTaskCommand::Reassign {
                id,
                key,
                expected_version,
                expected_owner,
                to,
                reason,
            } => (
                Method::POST,
                format!("tasks/{}/reassign", encoded(id)),
                Some(
                    json!({"client_takeover_id":key,"expected_task_version":expected_version,"expected_owner_session_id":expected_owner,"to_session_id":to,"reason":reason}),
                ),
            ),
            CollabTaskCommand::Takeovers { id, cursor, limit } => (
                Method::GET,
                query(
                    &format!("tasks/{}/takeovers", encoded(id)),
                    &[
                        ("cursor", cursor.clone()),
                        ("limit", limit.map(|n| n.to_string())),
                    ],
                ),
                None,
            ),
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
        CollabCommand::Question { command } => {
            match command {
                CollabQuestionCommand::Ask {
                    task,
                    key,
                    title,
                    body,
                    recipient,
                    recipient_id,
                    option,
                    task_version,
                    base_sha,
                    head_sha,
                } => {
                    if (recipient == "repo_writers") != recipient_id.is_none() {
                        return Err(AppError::Usage("recipient user/session requires --recipient-id; repo_writers must omit it".into()));
                    }
                    (
                        Method::POST,
                        format!("tasks/{}/questions", encoded(task)),
                        Some(
                            json!({"client_question_id":key,"recipient":{"kind":recipient,"id":recipient_id},"title":title,"body":body,"options":option,"context":{"task_version":task_version,"base_sha":base_sha,"head_sha":head_sha}}),
                        ),
                    )
                }
                CollabQuestionCommand::List {
                    task,
                    status,
                    cursor,
                    limit,
                } => (
                    Method::GET,
                    query(
                        &format!("tasks/{}/questions", encoded(task)),
                        &[
                            ("status", status.clone()),
                            ("cursor", cursor.clone()),
                            ("limit", limit.map(|n| n.to_string())),
                        ],
                    ),
                    None,
                ),
                CollabQuestionCommand::View { id } => {
                    (Method::GET, format!("questions/{}", encoded(id)), None)
                }
                CollabQuestionCommand::Answer {
                    id,
                    key,
                    expected_version,
                    context_hash,
                    body,
                    option_id,
                } => (
                    Method::POST,
                    format!("questions/{}/answer", encoded(id)),
                    Some(
                        json!({"client_answer_id":key,"expected_version":expected_version,"acknowledged_context_hash":context_hash,"body":body,"option_id":option_id}),
                    ),
                ),
                CollabQuestionCommand::Cancel {
                    id,
                    expected_version,
                } => (
                    Method::POST,
                    format!("questions/{}/cancel", encoded(id)),
                    Some(json!({"expected_version":expected_version})),
                ),
            }
        }
        CollabCommand::WorkPackage { command } => match command {
            CollabWorkPackageCommand::List {
                task_id,
                cursor,
                limit,
            } => (
                Method::GET,
                query(
                    &format!("tasks/{}/work-packages", encoded(task_id)),
                    &[
                        ("cursor", cursor.clone()),
                        ("limit", limit.map(|n| n.to_string())),
                    ],
                ),
                None,
            ),
            CollabWorkPackageCommand::Create {
                task_id,
                key,
                goal,
                scope,
                commit,
                expected_task_version,
                artifact,
                completed,
                next_step,
                open_item,
            } => {
                // Optional scalars are omitted rather than sent as null: the
                // schema types goal and scope as plain strings.
                let mut body = json!({
                    "client_package_id": key,
                    "artifacts": artifact,
                    "completed": completed,
                    "next_steps": next_step,
                    "open_items": open_item,
                });
                if let Some(goal) = goal {
                    body["goal"] = json!(goal);
                }
                if let Some(scope) = scope {
                    body["scope"] = json!(scope);
                }
                if let Some(commit) = commit {
                    body["commit_sha"] = json!(commit);
                }
                if let Some(version) = expected_task_version {
                    body["expected_task_version"] = json!(version);
                }
                (
                    Method::POST,
                    format!("tasks/{}/work-packages", encoded(task_id)),
                    Some(body),
                )
            }
            CollabWorkPackageCommand::View { id } => {
                (Method::GET, format!("work-packages/{}", encoded(id)), None)
            }
        },
        CollabCommand::Dependency { command } => match command {
            CollabDependencyCommand::List { task_id } => (
                Method::GET,
                format!("tasks/{}/dependencies", encoded(task_id)),
                None,
            ),
            CollabDependencyCommand::Add {
                task_id,
                depends_on_id,
                key,
                expected_graph_version,
            } => (
                Method::POST,
                format!("tasks/{}/dependencies", encoded(task_id)),
                Some(json!({
                    "client_request_id": key,
                    "depends_on_task_id": depends_on_id,
                    "expected_graph_version": expected_graph_version,
                })),
            ),
            CollabDependencyCommand::Remove {
                task_id,
                depends_on_id,
                key,
                expected_graph_version,
            } => (
                Method::DELETE,
                format!(
                    "tasks/{}/dependencies/{}",
                    encoded(task_id),
                    encoded(depends_on_id)
                ),
                Some(json!({
                    "client_request_id": key,
                    "expected_graph_version": expected_graph_version,
                })),
            ),
        },
        CollabCommand::Decision { command } => match command {
            CollabDecisionCommand::List {
                task_id,
                cursor,
                limit,
            } => (
                Method::GET,
                query(
                    &format!("tasks/{}/decisions", encoded(task_id)),
                    &[
                        ("cursor", cursor.clone()),
                        ("limit", limit.map(|n| n.to_string())),
                    ],
                ),
                None,
            ),
            CollabDecisionCommand::Create {
                task_id,
                key,
                title,
                rationale,
                question_id,
                answer_id,
                task_version,
                base_sha,
                head_sha,
            } => {
                let mut body = json!({
                    "client_decision_id": key,
                    "title": title,
                    "rationale": rationale,
                });
                if let Some(question_id) = question_id {
                    body["question_id"] = json!(question_id);
                }
                if let Some(answer_id) = answer_id {
                    body["answer_id"] = json!(answer_id);
                }
                // `context` is optional and nullable; send it only when one of its fields is set.
                if task_version.is_some() || base_sha.is_some() || head_sha.is_some() {
                    body["context"] = json!({
                        "task_version": task_version,
                        "base_sha": base_sha,
                        "head_sha": head_sha,
                    });
                }
                (
                    Method::POST,
                    format!("tasks/{}/decisions", encoded(task_id)),
                    Some(body),
                )
            }
            CollabDecisionCommand::View { id } => {
                (Method::GET, format!("decisions/{}", encoded(id)), None)
            }
        },
        CollabCommand::Inbox { user, since, limit } => (
            Method::GET,
            query(
                if *user { "inbox/user" } else { "inbox" },
                &[
                    ("since", since.clone()),
                    ("limit", limit.map(|n| n.to_string())),
                ],
            ),
            None,
        ),
        CollabCommand::Condition { command } => match command {
            CollabConditionCommand::List { task_id } => (
                Method::GET,
                format!("tasks/{}/completion-conditions", encoded(task_id)),
                None,
            ),
            CollabConditionCommand::Set { task_id, source } => (
                Method::PUT,
                format!("tasks/{}/completion-conditions", encoded(task_id)),
                Some(read_conditions_body(source)?),
            ),
            CollabConditionCommand::Confirm {
                task_id,
                condition_id,
                key,
                note,
            } => {
                let mut body = json!({"client_request_id": key});
                if let Some(note) = note {
                    body["note"] = json!(note);
                }
                (
                    Method::POST,
                    format!(
                        "tasks/{}/completion-conditions/{}/confirmations",
                        encoded(task_id),
                        encoded(condition_id)
                    ),
                    Some(body),
                )
            }
        },
        CollabCommand::Budget { command } => match command {
            CollabBudgetCommand::View { task_id } => (
                Method::GET,
                format!("tasks/{}/budget", encoded(task_id)),
                None,
            ),
        },
        CollabCommand::Usage { command } => match command {
            CollabUsageCommand::List {
                task_id,
                unit,
                source,
                cursor,
                limit,
            } => (
                Method::GET,
                query(
                    &format!("tasks/{}/usage", encoded(task_id)),
                    &[
                        ("unit", unit.clone()),
                        ("source", source.clone()),
                        ("cursor", cursor.clone()),
                        ("limit", limit.map(|n| n.to_string())),
                    ],
                ),
                None,
            ),
            CollabUsageCommand::Report {
                task_id,
                key,
                unit,
                source,
                amount,
                period_start,
                period_end,
            } => (
                Method::POST,
                format!("tasks/{}/usage", encoded(task_id)),
                Some(json!({
                    "client_usage_id": key,
                    "unit": unit,
                    "source": source,
                    "amount": amount,
                    "period_start": period_start,
                    "period_end": period_end,
                })),
            ),
        },
        CollabCommand::Rules { git_ref, path } => (
            Method::GET,
            query("rules", &[("ref", git_ref.clone()), ("path", path.clone())]),
            None,
        ),
        CollabCommand::Overview { sections, limit } => (
            Method::GET,
            query(
                "overview",
                &[
                    ("sections", sections.clone()),
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
