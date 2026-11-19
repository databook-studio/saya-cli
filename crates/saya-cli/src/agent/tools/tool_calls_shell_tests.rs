//! The `shell` half of the shared detail seam: the command it ran, stripped of
//! secret-shaped values and bounded, the way the `write` half names its path.
//!
//! The bound and the stripper are pinned here beside the projection; the
//! process-level `-p --format ndjson` lines are pinned in
//! `tests/oneshot_ndjson.rs`.

use super::tool_call_detail;

/// The detail of a `shell` call, or `None` when it carries no command.
fn shell_detail(command: &str) -> Option<String> {
    tool_call_detail("shell", &serde_json::json!({ "command": command }))
}

/// A command the model authored reaches the line as it was written.
#[test]
fn a_shell_request_names_the_command_it_will_run() {
    assert_eq!(
        shell_detail("echo hi").as_deref(),
        Some("echo hi"),
        "a shell request names its command"
    );
}

/// A blank or missing command exposes no detail, exactly as a `write` with no
/// path exposes none.
#[test]
fn a_shell_request_without_a_command_has_no_detail() {
    assert_eq!(shell_detail(""), None, "an empty command has no detail");
    assert_eq!(
        tool_call_detail("shell", &serde_json::json!({})),
        None,
        "a call with no command argument has no detail"
    );
}

/// One newline would split the line in two for a reader consuming one JSON
/// object per line; the flattened form keeps it a single line.
#[test]
fn newlines_are_flattened_into_one_line() {
    assert_eq!(
        shell_detail("set -e\necho hi  ").as_deref(),
        Some("set -e⏎echo hi  "),
        "every newline becomes the one visible marker, and nothing else moves"
    );
}

/// The stripper Saya already journals shell commands through is the one this
/// line is built from: a secret-shaped assignment never reaches it.
#[test]
fn the_command_is_the_shared_stripper_s_own_output() {
    let command = "export DB_PASSWORD=planted-shell-secret; echo hi";
    let detail = shell_detail(command).expect("a shell request names its command");
    assert_eq!(
        detail,
        saya_types::redact(command),
        "the detail is the shared stripper's output, not a second stripper"
    );
    assert!(
        !detail.contains("planted-shell-secret"),
        "the secret value must not survive: {detail:?}"
    );
}

/// An over-bound command is cut to 512 bytes of UTF-8 on a character
/// boundary, and the ellipsis says it was cut.
#[test]
fn an_over_bound_command_keeps_512_bytes_of_whole_characters() {
    let command = format!("echo {}", "é".repeat(2_500));
    let detail = shell_detail(&command).expect("a shell request names its command");
    let kept = detail
        .strip_suffix('…')
        .expect("a cut detail says it was cut");
    assert!(kept.len() <= 512, "kept {} bytes", kept.len());
    assert_eq!(
        kept.len(),
        511,
        "507 bytes of room fits 253 whole 2-byte chars"
    );
    assert!(
        kept.ends_with('é'),
        "the cut lands on a character boundary: {kept:?}"
    );
    assert!(
        command.starts_with(kept),
        "the kept bytes are the command's own opening"
    );
}

/// A command that fits is not marked: the ellipsis means cut, never decoration.
#[test]
fn a_command_inside_the_bound_is_carried_whole() {
    let command = "echo {}".replace("{}", &"x".repeat(505));
    assert_eq!(command.len(), 510);
    assert_eq!(
        shell_detail(&command).as_deref(),
        Some(command.as_str()),
        "a command inside the bound is carried whole"
    );
}
