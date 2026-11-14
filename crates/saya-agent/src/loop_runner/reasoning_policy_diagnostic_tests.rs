//! What a non-zero exit means. An ordinary "no" (no match, a difference, a
//! false condition) is not a diagnostic surprise; a command that never reached
//! its tests is. `"error": null` is not a failure at all.

use super::{diagnostic, effort, shell, tool_result};
use crate::{ChatMessage, ReasoningEffort};

fn shell_result(command: &str, exit_code: i64, stdout: &str, stderr: &str) -> Vec<ChatMessage> {
    vec![
        ChatMessage::text("user", "task"),
        shell("call-1", command),
        tool_result(
            "call-1",
            &serde_json::json!({
                "exit_code": exit_code,
                "stdout": stdout,
                "stderr": stderr,
            })
            .to_string(),
        ),
    ]
}

/// A non-zero exit that only means "the answer is no" is not a diagnostic
/// surprise: grep/rg found no match, diff found a difference, or `test` found
/// the condition false. The model already knows what to do next.
#[test]
fn ordinary_nonzero_exits_are_not_diagnostic() {
    for command in [
        "grep -q needle file",
        "rg needle src",
        "diff a b",
        "test -f build/output",
        "[ -f build/output ]",
        "cd repo && grep -q needle file",
    ] {
        let messages = shell_result(command, 1, "", "");
        assert_eq!(
            effort(&messages, false, false),
            ReasoningEffort::Low,
            "{command} exit 1 is an ordinary no"
        );
        assert!(
            !diagnostic(&messages, false),
            "{command} exit 1 must not arm the next escalation"
        );
    }
}

/// The same commands use a second exit code for a real error — a grep syntax
/// error, a diff that could not read a file, `test` with invalid usage. That
/// is the compiler telling the model something it did not know.
#[test]
fn a_real_error_from_an_ordinary_command_is_diagnostic() {
    for command in [
        "grep -q needle file",
        "rg needle src",
        "diff a b",
        "test -f build/output",
        "[ -f build/output ]",
        "grep -q needle file && cargo build",
    ] {
        let messages = shell_result(command, 2, "", "usage error");
        assert_eq!(
            effort(&messages, false, false),
            ReasoningEffort::High,
            "{command} exit 2 is a diagnostic failure"
        );
        assert!(diagnostic(&messages, false), "{command}");
    }
}

/// A test command that never reached its tests — the compiler or the test
/// collector failed — is diagnostic. A failing suite is not.
#[test]
fn a_test_command_that_cannot_compile_or_collect_is_diagnostic() {
    for (command, exit_code, output) in [
        (
            "cargo test --workspace",
            101,
            "error[E0308]: mismatched types",
        ),
        (
            "cargo nextest run --workspace",
            101,
            "error: could not compile `saya` (test \"agent_loop\")",
        ),
        ("pytest -q", 2, "ERROR collecting tests/test_agent.py"),
        ("python3 -m pytest -q", 4, ""),
        ("pytest", 5, ""),
    ] {
        let messages = shell_result(command, exit_code, "", output);
        assert_eq!(
            effort(&messages, false, false),
            ReasoningEffort::High,
            "{command} exit {exit_code} never ran the tests"
        );
        assert!(diagnostic(&messages, false), "{command}: {output}");
    }
}

#[test]
fn a_failing_pytest_suite_stays_light() {
    let messages = shell_result("pytest -q", 1, "1 failed, 7 passed", "");
    assert_eq!(effort(&messages, false, false), ReasoningEffort::Low);
    assert!(!diagnostic(&messages, false));
}

/// `"error": null` is the absence of an error, not a failure.
#[test]
fn a_null_error_field_is_not_a_failure() {
    let messages = vec![
        ChatMessage::text("user", "read it"),
        shell("call-1", "cat file"),
        tool_result("call-1", r#"{"error":null,"exit_code":0}"#),
    ];
    assert_eq!(effort(&messages, false, false), ReasoningEffort::Low);
    assert!(!diagnostic(&messages, false));
}
