//! Printing for the CLI half, and the small JSON helpers the TUI shares.

use std::sync::atomic::{AtomicBool, Ordering};

use comfy_table::{presets::UTF8_FULL, Table};
use serde_json::Value;

// The output mode is chosen once from --json and then read by every read-only
// command. A process-wide flag (like a logger's verbosity) avoids threading a
// `json: bool` through every signature and call site — it's configuration, not a
// per-function argument.
// ponytail: global flag; make it a parameter if two output modes are ever needed
// at once. One process per command, so not yet.
static JSON_OUTPUT: AtomicBool = AtomicBool::new(false);

pub fn set_json_output(on: bool) {
    JSON_OUTPUT.store(on, Ordering::Relaxed);
}

pub fn json_output() -> bool {
    JSON_OUTPUT.load(Ordering::Relaxed)
}

/// Print the raw API JSON. Its shape belongs to the server, not to us: scripts get
/// exactly what the API sent, including an empty `[]` rather than a "No items."
/// line that isn't valid JSON.
pub fn print_json(value: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
    );
}

pub fn table(headers: &[&str], rows: Vec<Vec<String>>) {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_header(headers.iter().map(|h| h.to_string()));
    for row in rows {
        table.add_row(row);
    }
    println!("{table}");
}

/// A JSON field by pointer (e.g. "/status/phase") as a string; "-" when absent.
pub fn field(value: &Value, pointer: &str) -> String {
    match value.pointer(pointer) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(v) if !v.is_null() => v.to_string(),
        _ => "-".to_string(),
    }
}

/// The first line of `text`, cut to `width` with an ellipsis when it doesn't fit.
///
/// Cut here rather than letting the terminal clip it: a clipped path or name reads
/// as a complete, shorter one, and "…" is the only mark that says otherwise.
pub fn first_line(text: &str, width: usize) -> String {
    let line = text.lines().next().unwrap_or("");
    if line.chars().count() <= width {
        return line.to_string();
    }
    let keep = width.saturating_sub(1);
    line.chars().take(keep).collect::<String>() + "…"
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_missing_field_is_a_dash_not_the_word_null() {
        let v = json!({ "name": "web", "replicas": 3, "enabled": true });
        assert_eq!(field(&v, "/name"), "web");
        assert_eq!(field(&v, "/replicas"), "3");
        assert_eq!(field(&v, "/enabled"), "true");
        assert_eq!(field(&v, "/nope"), "-");
    }

    #[test]
    fn a_cut_line_says_it_was_cut() {
        assert_eq!(first_line("short", 10), "short");
        assert_eq!(first_line("a-very-long-name", 8), "a-very-…");
        // A multi-line cell must never break the row it sits in.
        assert_eq!(first_line("one\ntwo", 10), "one");
    }
}
