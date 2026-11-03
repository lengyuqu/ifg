//! Output rendering (design doc §5.4): human tables by default, server-raw
//! JSON for `--json` and `api`. M1 tables carry no color styling at all, so
//! `NO_COLOR`/`--no-color` are already honored trivially; color support for
//! richer views lands with the M2 resource commands.

use std::io::Write;

use comfy_table::{presets::ASCII_FULL_CONDENSED, Table as ComfyTable};

/// Write raw response bytes to stdout, followed by a newline.
pub fn print_bytes(bytes: &[u8]) {
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(bytes);
    let _ = stdout.write_all(b"\n");
    let _ = stdout.flush();
}

/// Write a JSON value pretty-printed, followed by a newline.
pub fn print_json(value: &serde_json::Value) {
    let bytes = serde_json::to_vec_pretty(value).unwrap_or_default();
    print_bytes(&bytes);
}

/// Render a simple ASCII table.
pub fn render_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut table = ComfyTable::new();
    table.load_preset(ASCII_FULL_CONDENSED);
    table.set_header(headers.iter().copied());
    for row in rows {
        table.add_row(row.iter().cloned());
    }
    table.to_string()
}

pub fn print_table(headers: &[&str], rows: &[Vec<String>]) {
    println!("{}", render_table(headers, rows));
}
