//! `--template` rendering (design doc §5.4): a deliberately small subset of
//! `gh`'s Go templates — `{{.field}}`, `{{.nested.field}}`, `{{.array.0}}`
//! and the literal escapes `\n` / `\t`.
//!
//! No `{{range}}`: list commands render the template once per item instead,
//! which covers the scripting cases without a template engine (and without
//! the dependency it would drag in).

use serde_json::Value;

/// Render `template` against `item`. Unknown paths render empty, mirroring
/// how a missing field in `gh -q` yields nothing.
pub fn render(template: &str, item: &Value) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&unescape(&rest[..start]));
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            // Unterminated action: treat the remainder literally.
            out.push_str(&unescape(&rest[start..]));
            return out;
        };
        let action = after[..end].trim();
        out.push_str(&render_action(action, item));
        rest = &after[end + 2..];
    }
    out.push_str(&unescape(rest));
    out
}

fn render_action(action: &str, item: &Value) -> String {
    // `{{.this}}` is the item itself; `{{.}}` too.
    let Some(path) = action.strip_prefix('.') else {
        return String::new();
    };
    let path = path.trim();
    if path.is_empty() || path == "this" {
        return scalar(item);
    }
    let mut current = item;
    for segment in path.split('.') {
        current = match current {
            Value::Object(map) => match map.get(segment) {
                Some(value) => value,
                None => return String::new(),
            },
            Value::Array(items) => match segment.parse::<usize>() {
                Ok(index) => match items.get(index) {
                    Some(value) => value,
                    None => return String::new(),
                },
                Err(_) => return String::new(),
            },
            _ => return String::new(),
        };
    }
    scalar(current)
}

/// Scalars print bare; objects/arrays print as compact JSON (useful enough
/// and never surprising).
fn scalar(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        other => other.to_string(),
    }
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn renders_paths_and_escapes() {
        let item = json!({
            "number": 12,
            "title": "Fix the thing",
            "author": {"username": "alice"},
            "labels": ["bug", "p1"],
            "draft": false,
            "body": null,
        });
        assert_eq!(
            render("{{.number}}: {{.title}}", &item),
            "12: Fix the thing"
        );
        assert_eq!(render("{{.author.username}}", &item), "alice");
        assert_eq!(render("{{.labels.0}}/{{.labels.1}}", &item), "bug/p1");
        assert_eq!(render("{{.draft}}", &item), "false");
        assert_eq!(render("{{.body}}", &item), "");
        assert_eq!(render("x{{.missing.deep}}y", &item), "xy");
        assert_eq!(render("a\\tb\\nc", &item), "a\tb\nc");
        assert_eq!(render("{{.}}", &json!("plain")), "plain");
        // Unterminated action stays literal.
        assert_eq!(render("oops {{.title", &item), "oops {{.title");
    }
}
