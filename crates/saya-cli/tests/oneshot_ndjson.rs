//! `saya -p --format ndjson` names the shell command the model ran, the way the
//! `write` line already names its path: a headless observer (a CI log, a lane
//! audit) must be able to see what ran, secret-stripped and bounded.
//!
//! The four lines pinned here are `tool_requested` for a plain command, that
//! same line for a command carrying a secret-shaped value, that line for an
//! over-bound command, and `tool_denied` for a command a deny rule refused.
#![cfg(unix)]

#[path = "oneshot_ndjson/provider.rs"]
mod provider;
#[path = "oneshot_ndjson/support.rs"]
mod support;

use provider::{Round, event, ndjson, scripted};
use serde_json::json;
use support::{repository, saya, temp_root, text};

/// The command the deny-rule case refuses: the rule names its program.
const DENIED_COMMAND: &str = "echo hi";

/// A command carrying an assignment the shared stripper replaces.
const SECRET_COMMAND: &str = "export DB_PASSWORD=planted-shell-secret; echo hi";

/// The `-p` arguments every case here shares: ndjson lines, bypass consent (no
/// operator is present), any extra flags, then the one task `-p` takes as its
/// own value.
fn args<'a>(task: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec!["--format", "ndjson", "--approval-mode", "bypass"];
    args.extend_from_slice(extra);
    args.push("-p");
    args.push(task);
    args
}

/// A run whose model calls `shell` once with `command` and then answers.
fn run_shell(
    root: &std::path::Path,
    repo: &std::path::Path,
    command: &str,
    extra: &[&str],
) -> String {
    let command = command.to_owned();
    let provider = scripted(vec![
        Box::new(move || Round::Tool {
            name: "shell",
            arguments: json!({"command": command, "description": "run it", "sandbox": false}),
        }),
        Box::new(|| Round::Text("ran it")),
    ]);
    let output = saya(root, repo, &provider, &args("run one command", extra));
    let (stdout, stderr) = text(&output);
    assert!(
        output.status.success(),
        "the one-shot run completes: status={:?} stdout={stdout} stderr={stderr}",
        output.status
    );
    stdout
}

/// The `tool_requested` line for `shell` names the command it will run.
#[test]
fn ndjson_oneshot_names_the_shell_command_it_requested() {
    let root = temp_root("requested");
    let repo = repository(&root);
    let stdout = run_shell(&root, &repo, "echo hi", &[]);
    let lines = ndjson(&stdout);
    let requested = event(&lines, "tool_requested", "shell")
        .unwrap_or_else(|| panic!("the stream carries the shell request: {stdout}"));
    assert_eq!(
        requested["detail"], "echo hi",
        "the requested line names the command it ran: {requested}"
    );
}

/// A secret-shaped value the shared stripper removes never reaches the line.
#[test]
fn ndjson_oneshot_strips_a_secret_shape_from_the_shell_detail() {
    let root = temp_root("stripped");
    let repo = repository(&root);
    let stdout = run_shell(&root, &repo, SECRET_COMMAND, &[]);
    let lines = ndjson(&stdout);
    let requested = event(&lines, "tool_requested", "shell")
        .unwrap_or_else(|| panic!("the stream carries the shell request: {stdout}"));
    let detail = requested["detail"]
        .as_str()
        .unwrap_or_else(|| panic!("the shell request carries a detail: {requested}"));
    assert!(
        !detail.contains("planted-shell-secret"),
        "the detail must not carry the secret value: {detail:?}"
    );
    assert_eq!(
        detail,
        saya_types::redact(SECRET_COMMAND),
        "the detail is the shared stripper's own output"
    );
    assert!(
        detail.contains("echo hi"),
        "the rest of the command still reads: {detail:?}"
    );
}

/// An over-bound command is cut to 512 bytes of UTF-8 on a char boundary, with
/// the ellipsis saying so.
#[test]
fn ndjson_oneshot_bounds_a_long_shell_detail_on_a_char_boundary() {
    let root = temp_root("bounded");
    let repo = repository(&root);
    // 5 005 bytes: the program, then 2 500 two-byte characters, so a byte-wise
    // cut at the bound would land inside one of them.
    let command = format!("echo {}", "é".repeat(2_500));
    assert_eq!(
        command.len(),
        5_005,
        "the fixture command is over the bound"
    );
    let stdout = run_shell(&root, &repo, &command, &[]);
    let lines = ndjson(&stdout);
    let requested = event(&lines, "tool_requested", "shell")
        .unwrap_or_else(|| panic!("the stream carries the shell request: {stdout}"));
    let detail = requested["detail"]
        .as_str()
        .unwrap_or_else(|| panic!("the shell request carries a detail: {requested}"));
    assert!(
        detail.ends_with('…'),
        "a cut detail says it was cut: {}",
        detail.len()
    );
    let kept = detail
        .strip_suffix('…')
        .expect("the ellipsis is the cut marker");
    assert!(
        kept.len() <= 512,
        "the kept bytes stay within the bound: {}",
        kept.len()
    );
    assert!(
        detail.len() <= 512 + '…'.len_utf8(),
        "the detail is the bound plus the marker: {}",
        detail.len()
    );
    assert!(
        kept.ends_with('é'),
        "the cut lands on a character boundary, keeping whole characters"
    );
}

/// A deny rule refuses the call, and the denial line still names the command.
#[test]
fn ndjson_oneshot_names_the_shell_command_a_deny_rule_refused() {
    let root = temp_root("denied");
    let repo = repository(&root);
    let stdout = run_shell(&root, &repo, DENIED_COMMAND, &["--deny-command", "echo"]);
    let lines = ndjson(&stdout);
    let denied = event(&lines, "tool_denied", "shell")
        .unwrap_or_else(|| panic!("the stream carries the shell denial: {stdout}"));
    assert_eq!(
        denied["detail"], DENIED_COMMAND,
        "the denied line names the command it refused: {denied}"
    );
}
