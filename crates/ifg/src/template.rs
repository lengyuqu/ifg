//! `--template` rendering (design doc §5.4): a small subset of `gh`'s Go
//! templates — `{{.field}}`, `{{.nested.field}}`, `{{.array.0}}`,
//! `{{range .list}}…{{end}}` (nestable; objects iterate their values), and
//! the literal escapes `\n` / `\t`.
//!
//! Lists still render the template once per item; `range` exists for nested
//! arrays such as an issue's labels.

use serde_json::Value;

/// Render `template` against `item`.
pub fn render(template: &str, item: &Value) -> String {
    render_inner(template, item)
}

fn render_inner(template: &str, ctx: &Value) -> String {
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
        let tail = &after[end + 2..];

        if let Some(path) = action.strip_prefix("range ") {
            let Some((body, remainder)) = split_block(tail) else {
                // Missing {{end}}: render the rest literally.
                out.push_str(&unescape(&rest[start..]));
                return out;
            };
            match resolve(path.trim(), ctx) {
                Some(Value::Array(items)) => {
                    for item in items {
                        out.push_str(&render_inner(&body, item));
                    }
                }
                Some(Value::Object(map)) => {
                    for value in map.values() {
                        out.push_str(&render_inner(&body, value));
                    }
                }
                Some(other) => out.push_str(&render_inner(&body, other)),
                None => {} // missing path: behave like an empty array
            }
            rest = remainder;
            continue;
        }

        out.push_str(&scalar(
            &resolve(action, ctx).cloned().unwrap_or(Value::Null),
        ));
        rest = tail;
    }
    out.push_str(&unescape(rest));
    out
}

/// Split a block body from its matching `{{end}}`, respecting nesting.
fn split_block(tail: &str) -> Option<(String, &str)> {
    let mut depth = 1usize;
    let mut cursor = 0usize;
    while let Some(rel) = tail[cursor..].find("{{") {
        let start = cursor + rel;
        let after = &tail[start + 2..];
        let end = after.find("}}")?;
        let action = after[..end].trim();
        let after_end = start + 2 + end + 2;
        if action.starts_with("range ") {
            depth += 1;
            cursor = after_end;
            continue;
        }
        if action == "end" {
            depth -= 1;
            if depth == 0 {
                return Some((tail[..start].to_string(), &tail[after_end..]));
            }
        }
        cursor = after_end;
    }
    None
}

/// Resolve `.`, `.field` or `.nested.field` / `.array.0` against `ctx`.
fn resolve<'a>(action: &str, ctx: &'a Value) -> Option<&'a Value> {
    let path = action.strip_prefix('.')?.trim();
    if path.is_empty() || path == "this" {
        return Some(ctx);
    }
    let mut current = ctx;
    for segment in path.split('.') {
        current = match current {
            Value::Object(map) => map.get(segment)?,
            Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
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

    #[test]
    fn ranges_over_arrays_and_objects() {
        let item = json!({
            "labels": ["bug", "p1"],
            "meta": {"a": 1, "b": 2},
        });
        assert_eq!(
            render("{{range .labels}}[{{.}}]{{end}}", &item),
            "[bug][p1]"
        );
        assert_eq!(
            render("labels: {{range .labels}}{{.}},{{end}}", &item),
            "labels: bug,p1,"
        );
        // Objects iterate their values.
        assert_eq!(render("{{range .meta}}{{.}}{{end}}", &item), "12");
        // Missing path renders nothing.
        assert_eq!(render("x{{range .nope}}y{{end}}z", &item), "xz");
    }

    #[test]
    fn nested_ranges_and_fields() {
        let item = json!({
            "files": [
                {"path": "a.rs", "labels": ["x"]},
                {"path": "b.rs", "labels": []}
            ]
        });
        assert_eq!(
            render(
                "{{range .files}}{{.path}}({{range .labels}}{{.}}{{end}}) {{end}}",
                &item
            ),
            "a.rs(x) b.rs() "
        );
    }

    #[test]
    fn unterminated_range_stays_literal() {
        let item = json!({"labels": ["a"]});
        assert_eq!(render("{{range .labels}}x", &item), "{{range .labels}}x");
    }
}
