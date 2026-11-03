//! Command-line definition.

use clap::Parser;

/// Work with IronForge repositories, issues, pull requests, CI and releases
/// from the terminal.
#[derive(Debug, Parser)]
#[command(name = "ifg", version, arg_required_else_help = true)]
pub struct Cli {}
