//! `ifg`: command-line client for IronForge.
//!
//! Subcommands (`auth`, `api`, `repo`, `issue`, `pr`, `ci`, `release`) land
//! from M1 onward; see `docs/architecture/ifg-project-design-2026-11.md` §5.

mod cli;

use clap::Parser;

fn main() {
    let _cli = cli::Cli::parse();
}
