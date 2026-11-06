//! Generate the man pages in `man/` from the clap definition.
//!
//! Run through `scripts/gen-man.sh` so the committed pages stay in sync with
//! `--help` (CI checks the diff).

use std::path::PathBuf;

use clap::CommandFactory;
use clap_mangen::Man;

#[path = "../src/cli.rs"]
mod cli;

fn main() -> std::io::Result<()> {
    let out: PathBuf = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("man"));
    std::fs::create_dir_all(&out)?;

    let root = cli::Cli::command();
    // Top-level page: name, synopsis and the subcommand list.
    write_page(&root, &out, "ifg.1")?;

    for subcommand in root.get_subcommands() {
        let name = subcommand.get_name();
        if matches!(name, "help") {
            continue;
        }
        // The page must identify as `ifg-<cmd>` (NAME) while the usage line
        // reads `ifg <cmd> …` like the real invocation.
        // clap wants 'static strings here; the generator is a short-lived
        // process, so leaking the two names is harmless.
        let page_name: &'static str = Box::leak(format!("ifg-{name}").into_boxed_str());
        let bin_name: &'static str = Box::leak(format!("ifg {name}").into_boxed_str());
        let named = subcommand
            .clone()
            .name(page_name)
            .display_name(page_name)
            .bin_name(bin_name)
            // Subcommands do not inherit `version`; the man header wants it.
            .version(env!("CARGO_PKG_VERSION"));
        let page = format!("ifg-{name}.1");
        write_page(&named, &out, &page)?;
    }
    println!("wrote man pages to {}", out.display());
    Ok(())
}

fn write_page(command: &clap::Command, dir: &std::path::Path, file: &str) -> std::io::Result<()> {
    let mut buffer = Vec::new();
    Man::new(command.clone()).render(&mut buffer)?;
    // clap_mangen emits trailing spaces in some roff lines. Normalize the
    // generated artifacts so git's whitespace check agrees with --check.
    let text = String::from_utf8(buffer)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let mut normalized = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let (body, newline) = line
            .strip_suffix('\n')
            .map_or((line, ""), |body| (body, "\n"));
        normalized.push_str(body.trim_end_matches([' ', '\t']));
        normalized.push_str(newline);
    }
    std::fs::write(dir.join(file), normalized)
}
