//! `ifg api …` (design doc §4.7): raw pass-through to `/api/v1/`, the
//! escape hatch for everything without a dedicated command.
//!
//! `ifg api <path> [-X METHOD] [-f KEY=VALUE]… [--input FILE] [--paginate]`
//!
//! GET fields become query parameters; other methods get a JSON body of
//! strings. `--paginate` merges the `data` arrays of every page into one
//! JSON array (D5: server field names, untouched).

use ifg_client::Method;
use serde_json::{Map, Value};

use super::bootstrap;
use crate::cli::{ApiArgs, Cli};
use crate::error::AppError;
use crate::output;

pub async fn run(args: &ApiArgs, cli: &Cli) -> Result<(), AppError> {
    if args.jq.is_some() && cli.template.is_some() {
        return Err(AppError::Usage(
            "--jq and --template are mutually exclusive".into(),
        ));
    }
    let ctx = bootstrap(cli)?;
    let client = &ctx.client;

    let method = parse_method(&args.method)?;
    let mut path = args.path.clone();
    let mut body: Option<Value> = None;

    let fields = parse_fields(&args.fields)?;
    if !fields.is_empty() {
        if method == Method::GET {
            append_query(&mut path, &fields);
        } else {
            body = Some(string_body(&fields));
        }
    }

    if let Some(file) = &args.input {
        if method == Method::GET {
            return Err(AppError::Usage("--input requires a non-GET -X".into()));
        }
        if body.is_some() {
            return Err(AppError::Usage(
                "--input and --field are mutually exclusive".into(),
            ));
        }
        body = Some(read_json_body(file)?);
    }

    if args.paginate && method != Method::GET {
        return Err(AppError::Usage("--paginate requires GET".into()));
    }

    if args.jq.is_some() && method != Method::GET {
        return Err(AppError::Usage("--jq requires GET".into()));
    }

    if args.paginate {
        let items: Vec<Value> = client.paginate(&path).collect().await?;
        if let Some(expression) = &args.jq {
            let bytes = serde_json::to_vec(&items)
                .map_err(|e| AppError::Other(format!("could not encode JSON: {e}")))?;
            filter_jq(expression, args.jq_raw, &bytes)?;
        } else {
            output::print_json(&Value::Array(items));
        }
    } else {
        let bytes = client.raw(method, &path, body.as_ref()).await?;
        if let Some(expression) = &args.jq {
            filter_jq(expression, args.jq_raw, &bytes)?;
        } else {
            output::print_bytes(&bytes);
        }
    }
    Ok(())
}

/// Run jq directly (no shell) with output inherited, so large results cannot
/// fill a pipe while JSON is written to stdin.
fn filter_jq(expression: &str, raw: bool, bytes: &[u8]) -> Result<(), AppError> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    // jq accepts a stream of JSON values, but API responses must be exactly
    // one JSON value; reject HTML, empty bodies and extra trailing data.
    serde_json::from_slice::<Value>(bytes)
        .map_err(|_| AppError::Other("--jq requires a JSON response".into()))?;
    let mut command = Command::new("jq");
    command.arg("-M");
    if raw {
        command.arg("-r");
    }
    let mut child = command
        .arg("--")
        .arg(expression)
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| {
            AppError::Other(format!(
                "could not start jq (install jq and add it to PATH): {e}"
            ))
        })?;
    let write_result = child.stdin.take().expect("piped stdin").write_all(bytes);
    let status = child
        .wait()
        .map_err(|e| AppError::Other(format!("could not wait for jq: {e}")))?;
    if !status.success() {
        return Err(AppError::Other(format!("jq failed ({status})")));
    }
    write_result.map_err(|e| AppError::Other(format!("could not send JSON to jq: {e}")))?;
    Ok(())
}

fn parse_method(method: &str) -> Result<Method, AppError> {
    match method.to_ascii_uppercase().as_str() {
        "GET" => Ok(Method::GET),
        "HEAD" => Ok(Method::HEAD),
        "POST" => Ok(Method::POST),
        "PUT" => Ok(Method::PUT),
        "PATCH" => Ok(Method::PATCH),
        "DELETE" => Ok(Method::DELETE),
        other => Err(AppError::Usage(format!("unknown method '{other}'"))),
    }
}

fn parse_fields(fields: &[String]) -> Result<Vec<(String, String)>, AppError> {
    fields
        .iter()
        .map(|field| {
            field
                .split_once('=')
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .ok_or_else(|| {
                    AppError::Usage(format!("invalid field '{field}', expected KEY=VALUE"))
                })
        })
        .collect()
}

/// Append `key=value` pairs as percent-encoded query parameters.
fn append_query(path: &mut String, fields: &[(String, String)]) {
    use url::form_urlencoded::Serializer;
    let mut serializer = Serializer::new(String::new());
    for (key, value) in fields {
        serializer.append_pair(key, value);
    }
    let query = serializer.finish();
    let separator = if path.contains('?') { '&' } else { '?' };
    path.push(separator);
    path.push_str(&query);
}

fn string_body(fields: &[(String, String)]) -> Value {
    let mut object = Map::new();
    for (key, value) in fields {
        object.insert(key.clone(), Value::String(value.clone()));
    }
    Value::Object(object)
}

fn read_json_body(file: &str) -> Result<Value, AppError> {
    let text = if file == "-" {
        use std::io::Read;
        let mut buffer = String::new();
        std::io::stdin()
            .read_to_string(&mut buffer)
            .map_err(|e| AppError::Other(format!("could not read stdin: {e}")))?;
        buffer
    } else {
        std::fs::read_to_string(file)
            .map_err(|e| AppError::Other(format!("could not read {file}: {e}")))?
    };
    serde_json::from_str(&text).map_err(|e| AppError::Other(format!("invalid JSON in {file}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_parse_into_pairs() {
        let fields = vec!["name=demo".to_string(), "private=false".to_string()];
        let parsed = parse_fields(&fields).unwrap();
        assert_eq!(parsed[0], ("name".into(), "demo".into()));
        assert!(parse_fields(&["no-equals".to_string()]).is_err());
    }

    #[test]
    fn query_is_appended_after_existing_params() {
        let mut path = "repos?state=open".to_string();
        append_query(&mut path, &[("page".into(), "2".into())]);
        assert_eq!(path, "repos?state=open&page=2");

        // application/x-www-form-urlencoded: space becomes '+'.
        let mut path = "repos".to_string();
        append_query(&mut path, &[("q".into(), "a b&c".into())]);
        assert_eq!(path, "repos?q=a+b%26c");
    }

    #[test]
    fn body_fields_are_strings() {
        let body = string_body(&[
            ("name".into(), "demo".into()),
            ("flag".into(), "false".into()),
        ]);
        assert_eq!(body["name"], "demo");
        assert_eq!(body["flag"], "false");
    }
}
