//! `ifg schema`: the command tree as one JSON document, for AI agents.
//!
//! Names, about text, arguments and help come from the clap definition at run
//! time, so they cannot drift from `--help`. Two facts live elsewhere:
//! `--json` support is read from `cmd::STRUCTURED_OUTPUT`, the table that
//! `validate_output` enforces, and the side effect of a leaf command is read
//! from `WRITE` and `DESTRUCTIVE` below (any leaf not listed is `read`).
//! Nothing here reads config, tokens or the network.

use clap::{Arg, ArgAction, Command as ClapCommand, CommandFactory};
use serde_json::{json, Value};

use crate::cli::Cli;
use crate::cmd::structured_output_supported;
use crate::error::{AppError, EXIT_AUTH, EXIT_GENERAL, EXIT_NOT_FOUND, EXIT_USAGE};

/// Shape version of the document (`schema_version`). Fields may be added
/// within a version; removing or retyping one requires a new version.
pub const SCHEMA_VERSION: u32 = 1;

/// Leaf commands that change server or local state, by clap path. Every leaf
/// not listed here or in `DESTRUCTIVE` is `read`. `api` is listed because its
/// `--method` can write, and `pr checkout` / `repo clone` / `release download`
/// because they write local files or branches.
const WRITE: &[&str] = &[
    "api",
    "auth login",
    "auth logout",
    "auth git-credential",
    "repo create",
    "repo edit",
    "repo clone",
    "repo collaborators add",
    "repo collaborators set",
    "repo collaborators remove",
    "issue create",
    "issue comment",
    "issue close",
    "issue reopen",
    "issue edit",
    "pr create",
    "pr edit",
    "pr close",
    "pr reopen",
    "pr comment",
    "pr review",
    "pr merge",
    "pr checkout",
    "pr reviewers add",
    "pr reviewers remove",
    "ci run",
    "ci cancel",
    "ci retry",
    "ci approve",
    "release create",
    "release edit",
    "release upload",
    "release download",
    "label create",
    "label edit",
    "milestone create",
    "milestone edit",
    "wiki create",
    "wiki edit",
    "org create",
    "org add-member",
    "org remove-member",
    "notifications read",
    "collab session start",
    "collab session heartbeat",
    "collab session end",
    "collab task create",
    "collab task update",
    "collab task claim",
    "collab task reassign",
    "collab question ask",
    "collab question answer",
    "collab question cancel",
    "collab message send",
    "collab message read",
    "collab handoff create",
    "collab handoff accept",
    "collab handoff cancel",
];

/// Leaf commands that delete data.
const DESTRUCTIVE: &[&str] = &[
    "repo delete",
    "release delete",
    "label delete",
    "milestone delete",
    "wiki delete",
    "notifications delete",
];

/// Print the document for `command_path` (empty: every command) to stdout.
pub(crate) fn run(command_path: &[String]) -> Result<(), AppError> {
    let mut root = Cli::command();
    // Built, so every argument has its resolved action and num_args.
    root.build();
    let document = document(&root, command_path)?;
    crate::output::print_json(&document);
    Ok(())
}

fn document(root: &ClapCommand, command_path: &[String]) -> Result<Value, AppError> {
    let globals = global_ids(root);
    let commands: Vec<Value> = if command_path.is_empty() {
        visible_subcommands(root)
            .map(|cmd| node(cmd, vec![cmd.get_name()], &globals))
            .collect()
    } else {
        let (cmd, path) = find(root, command_path)?;
        vec![node(cmd, path, &globals)]
    };
    let global_args: Vec<Value> = root
        .get_arguments()
        .filter(|arg| arg.is_global_set() && !is_builtin(arg))
        .map(arg_json)
        .collect();
    Ok(json!({
        "schema_version": SCHEMA_VERSION,
        "ifg_version": env!("CARGO_PKG_VERSION"),
        "commands": commands,
        "global_args": global_args,
        "exit_codes": exit_codes(),
    }))
}

/// One command and, recursively, its subcommands. `json_supported` and
/// `effect` are only meaningful for leaves, so groups carry `null`.
fn node<'a>(cmd: &'a ClapCommand, path: Vec<&'a str>, globals: &[&str]) -> Value {
    let children: Vec<&'a ClapCommand> = visible_subcommands(cmd).collect();
    let leaf = children.is_empty();
    let json_supported = leaf.then(|| structured_output_supported(&path));
    let effect = leaf.then(|| effect_of(&path));
    let subcommands: Vec<Value> = children
        .into_iter()
        .map(|child| {
            let mut child_path = path.clone();
            child_path.push(child.get_name());
            node(child, child_path, globals)
        })
        .collect();
    // Root globals are listed once in `global_args`; clap repeats them on
    // every subcommand after `build`, so they are dropped here.
    let args: Vec<Value> = cmd
        .get_arguments()
        .filter(|arg| {
            !is_builtin(arg) && !(arg.is_global_set() && globals.contains(&arg.get_id().as_str()))
        })
        .map(arg_json)
        .collect();
    json!({
        "path": path,
        "about": cmd.get_about().map(ToString::to_string),
        "args": args,
        "subcommands": subcommands,
        "json_supported": json_supported,
        "effect": effect,
    })
}

fn arg_json(arg: &Arg) -> Value {
    let action = arg.get_action();
    let takes_value = action.takes_values();
    let multiple = matches!(action, ArgAction::Append)
        || arg
            .get_num_args()
            .is_some_and(|range| range.max_values() > 1);
    let possible_values: Vec<String> = if takes_value {
        arg.get_possible_values()
            .into_iter()
            .filter(|value| !value.is_hide_set())
            .map(|value| value.get_name().to_string())
            .collect()
    } else {
        Vec::new()
    };
    json!({
        "name": arg.get_id().as_str(),
        "long": arg.get_long().map(|long| format!("--{long}")),
        "short": arg.get_short().map(|short| format!("-{short}")),
        "positional": arg.is_positional(),
        "required": arg.is_required_set(),
        "takes_value": takes_value,
        "multiple": multiple,
        "possible_values": (!possible_values.is_empty()).then_some(possible_values),
        "help": arg.get_help().map(ToString::to_string),
    })
}

fn exit_codes() -> Value {
    let meanings = [
        (0, "success"),
        (
            EXIT_GENERAL,
            "general failure: server or network error, I/O, or bad local config",
        ),
        (
            EXIT_USAGE,
            "usage error: bad arguments or flags, nothing ran",
        ),
        (
            EXIT_AUTH,
            "authentication needed or refused (no token, 401 or 403)",
        ),
        (EXIT_NOT_FOUND, "the requested object does not exist"),
    ];
    Value::Object(
        meanings
            .into_iter()
            .map(|(code, meaning)| (code.to_string(), Value::from(meaning)))
            .collect(),
    )
}

fn effect_of(path: &[&str]) -> &'static str {
    let name = path.join(" ");
    if DESTRUCTIVE.contains(&name.as_str()) {
        "destructive"
    } else if WRITE.contains(&name.as_str()) {
        "write"
    } else {
        "read"
    }
}

/// Subcommands that the schema lists. clap's implicit `help` is not a command.
fn visible_subcommands(cmd: &ClapCommand) -> impl Iterator<Item = &ClapCommand> {
    cmd.get_subcommands().filter(|sub| sub.get_name() != "help")
}

/// Names of the root's global arguments, as clap propagates them.
fn global_ids(root: &ClapCommand) -> Vec<&str> {
    root.get_arguments()
        .filter(|arg| arg.is_global_set() && !is_builtin(arg))
        .map(|arg| arg.get_id().as_str())
        .collect()
}

/// clap's own `-h/--help` and `-V/--version` are not part of the schema.
fn is_builtin(arg: &Arg) -> bool {
    matches!(arg.get_id().as_str(), "help" | "version")
}

/// Resolve a user-given path against the clap tree, returning the command
/// and its canonical path. Any unknown segment is a usage error.
fn find<'a>(
    root: &'a ClapCommand,
    command_path: &[String],
) -> Result<(&'a ClapCommand, Vec<&'a str>), AppError> {
    let mut current = root;
    let mut path = Vec::with_capacity(command_path.len());
    for name in command_path {
        let next = visible_subcommands(current).find(|cmd| cmd.get_name() == name.as_str());
        let Some(next) = next else {
            return Err(AppError::Usage(format!(
                "no command `{}`; run `ifg schema` to list every command",
                command_path.join(" ")
            )));
        };
        current = next;
        path.push(next.get_name());
    }
    Ok((current, path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::STRUCTURED_OUTPUT;
    use std::collections::BTreeSet;

    fn tree() -> ClapCommand {
        let mut root = Cli::command();
        root.build();
        root
    }

    /// Leaf paths read straight from the clap tree, independent of the
    /// document builder.
    fn clap_leaves(cmd: &ClapCommand, path: &str, out: &mut BTreeSet<String>) {
        let children: Vec<&ClapCommand> = visible_subcommands(cmd).collect();
        if children.is_empty() {
            if !path.is_empty() {
                out.insert(path.to_string());
            }
            return;
        }
        for child in children {
            let next = if path.is_empty() {
                child.get_name().to_string()
            } else {
                format!("{path} {}", child.get_name())
            };
            clap_leaves(child, &next, out);
        }
    }

    /// Leaf paths in the document, checking each leaf's fields on the way.
    fn document_leaves(node: &Value, out: &mut BTreeSet<String>) {
        let children = node["subcommands"].as_array().expect("subcommands");
        if children.is_empty() {
            let path: Vec<&str> = node["path"]
                .as_array()
                .expect("path")
                .iter()
                .map(|segment| segment.as_str().expect("segment"))
                .collect();
            assert!(node["json_supported"].is_boolean(), "{path:?}");
            assert!(
                matches!(
                    node["effect"].as_str(),
                    Some("read" | "write" | "destructive")
                ),
                "{path:?}"
            );
            out.insert(path.join(" "));
        } else {
            assert!(node["json_supported"].is_null(), "{node}");
            assert!(node["effect"].is_null(), "{node}");
        }
        for child in children {
            document_leaves(child, out);
        }
    }

    #[test]
    fn every_leaf_command_is_in_the_schema() {
        let root = tree();
        let mut expected = BTreeSet::new();
        clap_leaves(&root, "", &mut expected);
        let doc = document(&root, &[]).unwrap();
        let mut actual = BTreeSet::new();
        for node in doc["commands"].as_array().unwrap() {
            document_leaves(node, &mut actual);
        }
        assert!(expected.len() > 50, "the clap tree looks truncated");
        assert_eq!(actual, expected);
    }

    #[test]
    fn effect_table_names_only_existing_leaf_commands() {
        let root = tree();
        let mut leaves = BTreeSet::new();
        clap_leaves(&root, "", &mut leaves);
        for name in WRITE.iter().chain(DESTRUCTIVE) {
            assert!(
                leaves.contains(*name),
                "effect table names `{name}`, which is not a leaf command"
            );
        }
        let unique: BTreeSet<&str> = WRITE.iter().chain(DESTRUCTIVE).copied().collect();
        assert_eq!(
            unique.len(),
            WRITE.len() + DESTRUCTIVE.len(),
            "listed twice"
        );
    }

    #[test]
    fn structured_output_table_names_existing_commands() {
        let root = tree();
        let mut leaves = BTreeSet::new();
        clap_leaves(&root, "", &mut leaves);
        for entry in STRUCTURED_OUTPUT {
            let under = format!("{entry} ");
            assert!(
                leaves
                    .iter()
                    .any(|leaf| leaf == entry || leaf.starts_with(&under)),
                "structured-output table names `{entry}`, which matches no command"
            );
        }
    }

    #[test]
    fn json_support_follows_the_shared_table() {
        assert!(structured_output_supported(&["pr", "view"]));
        assert!(structured_output_supported(&["collab", "session", "start"]));
        assert!(structured_output_supported(&["notifications"]));
        assert!(!structured_output_supported(&["pr", "checkout"]));
        assert!(!structured_output_supported(&["version"]));
        assert!(!structured_output_supported(&["pr"]));
    }

    /// The exact `commands` value for `ifg schema pr merge`. A change here is
    /// a change to the 1.x document, so update it deliberately.
    fn snapshot_pr_merge() -> Value {
        let flag = |long: &str, name: &str, help: Option<&str>| {
            json!({
                "name": name,
                "long": long,
                "short": null,
                "positional": false,
                "required": false,
                "takes_value": false,
                "multiple": false,
                "possible_values": null,
                "help": help,
            })
        };
        json!([{
            "path": ["pr", "merge"],
            "about": "Merge a pull request",
            "args": [
                {
                    "name": "number",
                    "long": null,
                    "short": null,
                    "positional": true,
                    "required": true,
                    "takes_value": true,
                    "multiple": false,
                    "possible_values": null,
                    "help": null,
                },
                flag("--merge", "merge", None),
                flag("--squash", "squash", None),
                flag("--rebase", "rebase", None),
                flag("--auto", "auto", Some("Enable auto-merge instead of merging now")),
            ],
            "subcommands": [],
            "json_supported": true,
            "effect": "write",
        }])
    }

    #[test]
    fn pr_merge_subtree_snapshot() {
        let root = tree();
        let doc = document(&root, &["pr".into(), "merge".into()]).unwrap();
        assert_eq!(doc["commands"], snapshot_pr_merge());
    }

    #[test]
    fn unknown_path_is_a_usage_error() {
        let root = tree();
        let error = document(&root, &["pr".into(), "nope".into()]).unwrap_err();
        assert_eq!(error.exit_code(), EXIT_USAGE);
    }
}
