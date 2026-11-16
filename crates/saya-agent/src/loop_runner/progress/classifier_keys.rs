//! The keys and digests the classifier compares one call against the last:
//! what makes two calls the same call, and whether a result moved anything.

use serde_json::Value;

use crate::loop_runner::failure_key::{canonical_json, fnv1a64};

/// The key one read-shaped call is remembered under for progress: the path and
/// line range for `read`, the path for `list`, and the pattern and scope for
/// `grep`/`glob`.
pub(super) fn read_key(name: &str, arguments: &Value) -> String {
    match name {
        "read" => ranged_read_key(arguments),
        "list" => format!("list\u{0}{}", text(arguments, "path")),
        "grep" | "glob" => format!(
            "{name}\u{0}{}\u{0}{}",
            text(arguments, "pattern"),
            text(arguments, "path")
        ),
        _ => format!("{name}\u{0}{}", canonical_json(arguments)),
    }
}

/// The key one call is remembered under for the repeat detector. It names the
/// semantic target, not the incidental arguments: the same file range, the
/// same search, or the same command modulo whitespace.
pub(super) fn repeat_key(name: &str, arguments: &Value) -> String {
    match name {
        "read" => ranged_read_key(arguments),
        "list" => format!("list\u{0}{}", text(arguments, "path")),
        "grep" | "glob" => format!(
            "{name}\u{0}{}\u{0}{}",
            text(arguments, "pattern"),
            text(arguments, "path")
        ),
        "shell" | "run_command" => shell_key(name, arguments),
        _ => format!("{name}\u{0}{}", canonical_json(arguments)),
    }
}

/// A `read` call's target: the path and the line range it asked for. The
/// progress and repeat detectors share the shape, so a range is one target.
fn ranged_read_key(arguments: &Value) -> String {
    let start = arguments
        .get("start_line")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    let count = arguments
        .get("line_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    format!(
        "read\u{0}{}\u{0}{start}\u{0}{count}",
        text(arguments, "path")
    )
}

/// The key one shell command is remembered under: its command modulo
/// whitespace, plus the workdir that gives it its meaning. `description`,
/// `timeout` and `sandbox` describe the call, not the command.
pub(super) fn shell_key(name: &str, arguments: &Value) -> String {
    let Some(command) = arguments.get("command").and_then(Value::as_str) else {
        return format!("{name}\u{0}{}", canonical_json(arguments));
    };
    let workdir = arguments
        .get("workdir")
        .and_then(Value::as_str)
        .unwrap_or("");
    let command = command.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("{name}\u{0}{workdir}\u{0}{command}")
}

/// A result's output, normalized so incidental whitespace does not read as a
/// new outcome. The exit code is compared beside it.
pub(super) fn output_digest(result: &Value) -> u64 {
    let mut normalized = String::new();
    for field in ["stdout", "stderr"] {
        if let Some(text) = result.get(field).and_then(Value::as_str) {
            for word in text.split_whitespace() {
                normalized.push_str(word);
                normalized.push(' ');
            }
        }
    }
    fnv1a64(normalized.as_bytes())
}

/// Whether a result carried any non-whitespace output.
pub(super) fn has_output(result: &Value) -> bool {
    ["stdout", "stderr"].iter().any(|field| {
        result
            .get(*field)
            .and_then(Value::as_str)
            .is_some_and(|text| !text.trim().is_empty())
    })
}

/// Whether a search result carried at least one match.
pub(super) fn has_matches(result: &Value) -> bool {
    result
        .get("matches")
        .and_then(Value::as_array)
        .is_some_and(|matches| !matches.is_empty())
}

fn text<'a>(arguments: &'a Value, field: &str) -> &'a str {
    arguments.get(field).and_then(Value::as_str).unwrap_or("")
}
