//! Output rendering (design doc §5.4): tables for humans, server-raw JSON
//! for `--json`/pipes. Colors, the pager and relative timestamps only switch
//! on when stdout is a terminal, so piped output stays plain and scriptable.

use std::io::{IsTerminal, Write};
use std::sync::OnceLock;

use comfy_table::{presets::ASCII_FULL_CONDENSED, Cell, Color, Table as ComfyTable};

/// Terminal capabilities, decided once per process.
#[derive(Debug, Clone, Copy)]
pub struct Capabilities {
    /// stdout is a terminal (colors, pager, relative timestamps).
    pub terminal: bool,
    /// Colors are on: terminal and neither `--no-color` nor `NO_COLOR`.
    pub color: bool,
    /// Row count from which the pager kicks in.
    pub pager_at_rows: u64,
}

static CAPS: OnceLock<Capabilities> = OnceLock::new();

/// Decide and cache the output capabilities from the global flags.
pub fn init(no_color: bool) {
    let terminal = std::io::stdout().is_terminal();
    let color = terminal && ansi_terminal() && !no_color && std::env::var_os("NO_COLOR").is_none();
    // `LINES` is advisory; 40 is a sane fallback for "one screenful".
    let rows = std::env::var("LINES")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|lines| *lines > 8)
        .unwrap_or(40);
    let _ = CAPS.set(Capabilities {
        terminal,
        color,
        pager_at_rows: rows,
    });
}

/// Whether the terminal interprets ANSI escapes. Windows consoles only do so
/// when virtual-terminal processing is enabled; enabling it needs
/// `SetConsoleMode` and thus `unsafe`, which the workspace forbids. Require a
/// known-good terminal instead (Windows Terminal sets `WT_SESSION`, VS Code
/// and mintty/Git Bash set `TERM_PROGRAM`/`TERM`): plain cmd.exe falls back to
/// plain output, equivalent to `--no-color`.
fn ansi_terminal() -> bool {
    #[cfg(windows)]
    {
        ["WT_SESSION", "TERM_PROGRAM", "TERM"]
            .iter()
            .any(|name| std::env::var_os(name).is_some())
    }
    #[cfg(not(windows))]
    {
        true
    }
}

fn caps() -> Capabilities {
    CAPS.get().copied().unwrap_or(Capabilities {
        terminal: false,
        color: false,
        pager_at_rows: 40,
    })
}

/// Colorize a cell when it is a known state word (cheap, centralized
/// prettification; unknown text passes through untouched).
fn colorize(text: &str, color_enabled: bool) -> Cell {
    let cell = Cell::new(text);
    if !color_enabled {
        return cell;
    }
    let color = match text {
        "open" | "merged" | "running" | "success" => Some(Color::Green),
        "closed" | "failed" | "failure" | "canceled" | "cancelled" => Some(Color::Red),
        "queued" | "pending" | "waiting" => Some(Color::Yellow),
        "draft" | "skipped" => Some(Color::DarkGrey),
        _ => None,
    };
    match color {
        Some(color) => cell.fg(color),
        None => cell,
    }
}

/// A plain table. Cells that are exactly a known state word are colored when
/// the terminal supports it; long tables go through the pager.
pub fn print_table(headers: &[&str], rows: &[Vec<String>]) {
    let caps = caps();
    let mut table = ComfyTable::new();
    table.load_preset(ASCII_FULL_CONDENSED);
    table.set_header(headers.iter().map(Cell::new).collect::<Vec<_>>());
    for row in rows {
        table.add_row(
            row.iter()
                .map(|cell| colorize(cell, caps.color))
                .collect::<Vec<_>>(),
        );
    }
    let rendered = table.to_string();
    if caps.terminal && rows.len() as u64 >= caps.pager_at_rows {
        page(&rendered);
    } else {
        // Ignore EPIPE: `ifg … | head` closing the pipe is normal, not a crash.
        let _ = std::io::stdout().write_all(rendered.as_bytes());
        let _ = std::io::stdout().write_all(b"\n");
    }
}

/// Send large output through `$PAGER` (default `less -R`; no pager on Windows,
/// where `less` is usually absent and `more` mishandles ANSI escapes). Falls
/// back to a plain print when the pager cannot be spawned.
fn page(content: &str) {
    let pager = std::env::var("PAGER").ok().or_else(default_pager);
    let Some(pager) = pager else {
        print!("{content}");
        let _ = std::io::stdout().flush();
        return;
    };
    let mut parts = pager.split_whitespace();
    let (program, args) = match (parts.next(), parts.collect::<Vec<_>>()) {
        (Some(program), args) => (program, args),
        (None, _) => {
            print!("{content}");
            return;
        }
    };
    let child = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .spawn();
    match child {
        Ok(mut child) => {
            if let Some(stdin) = child.stdin.as_mut() {
                let _ = stdin.write_all(content.as_bytes());
                let _ = stdin.write_all(b"\n");
            }
            let _ = child.wait();
        }
        Err(_) => {
            print!("{content}");
            let _ = std::io::stdout().flush();
        }
    }
}

fn default_pager() -> Option<String> {
    #[cfg(windows)]
    {
        None
    }
    #[cfg(not(windows))]
    {
        Some("less -R".to_string())
    }
}

/// Print a unified diff: `+` green, `-` red, `@@` cyan on a terminal, and
/// paged when it is taller than the screen. Piped output stays plain.
pub fn print_diff(text: &str) {
    let caps = caps();
    let mut out = String::with_capacity(text.len() + 64);
    for line in text.lines() {
        if caps.color {
            if line.starts_with("+++") || line.starts_with("---") {
                out.push_str(line);
            } else if line.starts_with('+') {
                out.push_str("\x1b[32m");
                out.push_str(line);
                out.push_str("\x1b[0m");
            } else if line.starts_with('-') {
                out.push_str("\x1b[31m");
                out.push_str(line);
                out.push_str("\x1b[0m");
            } else if line.starts_with("@@") {
                out.push_str("\x1b[36m");
                out.push_str(line);
                out.push_str("\x1b[0m");
            } else {
                out.push_str(line);
            }
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    let lines = text.lines().count() as u64;
    if caps.terminal && lines >= caps.pager_at_rows {
        page(&out);
    } else {
        let _ = std::io::stdout().write_all(out.as_bytes());
    }
}

/// Render `items` through the `--template` subset, one line-group per item.
pub fn print_template(items: &[serde_json::Value], template: &str) {
    let mut out = String::new();
    for item in items {
        out.push_str(&crate::template::render(template, item));
        out.push('\n');
    }
    let _ = std::io::stdout().write_all(out.as_bytes());
}

/// Print a JSON value with two-space indent (EPIPE-tolerant).
pub fn print_json(value: &serde_json::Value) {
    let text = serde_json::to_string_pretty(value).unwrap_or_default();
    let _ = std::io::stdout().write_all(text.as_bytes());
    let _ = std::io::stdout().write_all(b"\n");
}

/// `ifg api` pass-through: the bytes matter, not the structure.
pub fn print_bytes(bytes: &[u8]) {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    let _ = handle.write_all(bytes);
    if !bytes.ends_with(b"\n") {
        let _ = handle.write_all(b"\n");
    }
    let _ = handle.flush();
}

/// Timestamps for humans: relative on a terminal ("3m ago"), absolute
/// (YYYY-MM-DD) in pipes and scripts (design doc §5.4).
pub fn human_time(iso: Option<&str>) -> String {
    let Some(iso) = iso else {
        return "—".to_string();
    };
    if caps().terminal {
        crate::time::relative(iso).unwrap_or_else(|| date_part(iso))
    } else {
        date_part(iso)
    }
}

fn date_part(iso: &str) -> String {
    iso.chars().take(10).collect()
}
