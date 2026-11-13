//! `ifg events list`: cursor reader of the repository event log.
//!
//! One read per run. `--since` is the exclusive cursor and `next_cursor` (in
//! the server object) is what the next call passes back. `timeout` is never
//! sent, so the server answers at once and the command never waits.

use serde_json::Value;

use super::bootstrap;
use crate::cli::{Cli, EventsCommand};
use crate::context::resolve_repo;
use crate::error::AppError;
use crate::output;

/// Longest summary shown in a table row; longer text is cut with an ellipsis.
const SUMMARY_CHARS: usize = 120;

pub async fn run(command: &EventsCommand, cli: &Cli) -> Result<(), AppError> {
    let EventsCommand::List {
        since,
        kinds,
        limit,
    } = command;
    validate_kinds(kinds)?;
    let ctx = bootstrap(cli)?;
    let repo = resolve_repo(cli.repo.as_deref(), &ctx.host)?;
    let response = ctx
        .client
        .events(&repo.owner, &repo.name)
        .list(*since, kinds, *limit)
        .await?;

    let cursor = response
        .get("next_cursor")
        .filter(|value| !value.is_null())
        .map(|value| output::sanitize(&cursor_text(value)));
    let events = response.get("events").and_then(Value::as_array);
    let structured = cli.json || cli.template.is_some();

    match since {
        Some(_) if events.is_none() => {
            eprintln!("ifg: warning: the response has no events array; use --json to see it");
        }
        Some(_) => {}
        None => {
            // Without a cursor the server only reports where to start reading.
            // Printing nothing would read as "nothing happened".
            match &cursor {
                Some(cursor) => eprintln!(
                    "note: no --since given, so {cursor} is the starting position, not an \
                     empty result; pass --since {cursor} to read the events after it"
                ),
                None => eprintln!(
                    "note: no --since given, so this is the starting position, not an empty result"
                ),
            }
        }
    }

    if structured {
        super::show(cli, &response);
        return Ok(());
    }

    match &cursor {
        Some(cursor) => eprintln!("next cursor: {cursor}"),
        None => eprintln!("ifg: warning: the response has no next_cursor"),
    }
    let events = events.map_or(&[][..], Vec::as_slice);
    if events.is_empty() {
        if let Some(since) = since {
            output::line!("No events after cursor {since}.");
        }
        return Ok(());
    }
    let rows: Vec<Vec<String>> = events.iter().map(event_row).collect();
    output::print_table(&["ID", "TIME", "TYPE", "PAYLOAD"], &rows);
    Ok(())
}

/// Each `--kind` becomes one entry of the server's comma-separated `types`,
/// so an entry containing a comma (or an empty one) would change the filter.
fn validate_kinds(kinds: &[String]) -> Result<(), AppError> {
    for kind in kinds {
        if kind.is_empty() || kind.contains(',') {
            return Err(AppError::Usage(format!(
                "invalid --kind '{kind}': give one event name, without commas"
            )));
        }
    }
    Ok(())
}

/// Render a cursor for humans and for the stderr line; numbers print bare.
fn cursor_text(cursor: &Value) -> String {
    match cursor {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// One table row: id, time, type and the payload on one line.
fn event_row(event: &Value) -> Vec<String> {
    // The server's event is `{id, type, created_at, payload, payload_origin}`;
    // the payload is whatever the event carried, so it is shown as compact
    // JSON rather than guessed at field by field.
    let id = match event.get("id") {
        Some(Value::Number(id)) => id.to_string(),
        Some(Value::String(id)) if !id.is_empty() => id.clone(),
        _ => "-".into(),
    };
    let payload = match event.get("payload") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Null) | None => event.to_string(),
        Some(other) => other.to_string(),
    };
    vec![
        id,
        output::human_time(text_field(event, &["created_at"]).as_deref()),
        text_field(event, &["type"]).unwrap_or_else(|| "-".into()),
        clip(&flatten(&payload), SUMMARY_CHARS),
    ]
}

/// First non-empty string among `names`.
fn text_field(value: &Value, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| match value.get(name)? {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        _ => None,
    })
}

/// Keep one line per event: newlines and tabs become spaces.
fn flatten(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if matches!(ch, '\n' | '\r' | '\t') {
                ' '
            } else {
                ch
            }
        })
        .collect()
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut clipped: String = text.chars().take(max.saturating_sub(1)).collect();
    clipped.push('…');
    clipped
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn kinds_reject_commas_and_empty_names() {
        assert!(validate_kinds(&["issue".into(), "pr".into()]).is_ok());
        assert!(validate_kinds(&["issue,pr".into()]).is_err());
        assert!(validate_kinds(&["".into()]).is_err());
    }

    #[test]
    fn row_shows_server_fields_and_falls_back() {
        let row = event_row(&json!({
            "id": 41,
            "type": "pull_request",
            "created_at": "2026-11-09T10:00:00Z",
            "payload": {"action": "opened", "number": 3},
            "payload_origin": "user_content"
        }));
        assert_eq!(row[0], "41");
        assert_eq!(row[2], "pull_request");
        assert_eq!(row[3], r#"{"action":"opened","number":3}"#);

        let unknown = event_row(&json!({"other": 9}));
        assert_eq!(unknown[0], "-");
        assert_eq!(unknown[2], "-");
        assert_eq!(unknown[3], r#"{"other":9}"#);
    }

    #[test]
    fn summary_stays_on_one_line() {
        assert_eq!(flatten("a\nb\tc"), "a b c");
        assert_eq!(clip("abcdef", 4), "abc…");
        assert_eq!(clip("abc", 4), "abc");
    }
}
