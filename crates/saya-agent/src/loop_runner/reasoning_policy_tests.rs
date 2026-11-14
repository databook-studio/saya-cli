use super::decide;
use crate::{ChatMessage, ReasoningEffort, ToolCall};

fn tool_result(id: &str, content: &str) -> ChatMessage {
    ChatMessage {
        role: "tool".into(),
        content: content.into(),
        tool_call_id: Some(id.into()),
        reasoning: None,
        reasoning_signature: None,
        ..Default::default()
    }
}

fn tool(content: &str) -> ChatMessage {
    tool_result("call-1", content)
}

fn assistant() -> ChatMessage {
    ChatMessage::text("assistant", "working")
}

fn shell(id: &str, command: &str) -> ChatMessage {
    ChatMessage {
        role: "assistant".into(),
        content: String::new(),
        tool_calls: vec![ToolCall {
            id: id.into(),
            name: "shell".into(),
            arguments: serde_json::json!({ "command": command }),
        }],
        reasoning: None,
        reasoning_signature: None,
        ..Default::default()
    }
}

fn effort(
    messages: &[ChatMessage],
    first_request: bool,
    previous_diagnostic_failure: bool,
) -> ReasoningEffort {
    decide(messages, first_request, previous_diagnostic_failure, None).effort
}

fn diagnostic(messages: &[ChatMessage], previous_diagnostic_failure: bool) -> bool {
    decide(messages, false, previous_diagnostic_failure, None).diagnostic_failure
}

#[test]
fn the_first_request_of_a_run_thinks_hard() {
    let messages = vec![ChatMessage::text("user", "find the answer")];
    assert_eq!(
        effort(&messages, true, false),
        ReasoningEffort::High,
        "a run's first request has no prior turn to build on"
    );
}

#[test]
fn a_clean_follow_up_thinks_lightly() {
    let messages = vec![
        ChatMessage::text("user", "find the answer"),
        assistant(),
        tool(r#"{"exit_code":0,"stdout":"ok"}"#),
    ];
    assert_eq!(effort(&messages, false, false), ReasoningEffort::Low);
    assert!(!diagnostic(&messages, false));
}

#[test]
fn a_tool_error_raises_the_effort() {
    let messages = vec![
        ChatMessage::text("user", "find the answer"),
        assistant(),
        tool(r#"{"error":"relation does not exist"}"#),
    ];
    assert_eq!(effort(&messages, false, false), ReasoningEffort::High);
    assert!(diagnostic(&messages, false));
}

#[test]
fn a_nonzero_shell_exit_raises_the_effort() {
    let messages = vec![
        ChatMessage::text("user", "build it"),
        assistant(),
        tool(r#"{"exit_code":2,"stdout":"","stderr":"boom"}"#),
    ];
    assert_eq!(effort(&messages, false, false), ReasoningEffort::High);
    assert!(diagnostic(&messages, false));
}

/// A failing test suite is the task's normal starting state, not a diagnostic
/// surprise: the model already knows what to do next, so the effort stays low.
#[test]
fn a_failing_test_command_stays_light() {
    let messages = vec![
        ChatMessage::text("user", "fix the failing tests"),
        shell("call-1", "cargo test --workspace"),
        tool(r#"{"exit_code":1,"stdout":"test result: FAILED"}"#),
    ];
    assert_eq!(effort(&messages, false, false), ReasoningEffort::Low);
    assert!(
        !diagnostic(&messages, false),
        "an expected red suite does not arm the next escalation"
    );
}

/// The same non-zero exit from a build is a diagnostic failure: the compiler
/// told the model something it did not know, so the next request thinks hard.
#[test]
fn a_failing_build_command_raises_the_effort() {
    let messages = vec![
        ChatMessage::text("user", "build it"),
        shell("call-1", "cargo build"),
        tool(r#"{"exit_code":101,"stderr":"error[E0308]"}"#),
    ];
    assert_eq!(effort(&messages, false, false), ReasoningEffort::High);
    assert!(diagnostic(&messages, false));
}

/// Every common test invocation is recognised however it is wrapped: a `cd`,
/// a redirection, a pipe to a pager, or a runner flag.
#[test]
fn wrapped_test_commands_stay_light() {
    for command in [
        "cd repo && cargo test --workspace 2>&1 | tail -20",
        "cargo nextest run --workspace",
        "cargo test -- --nocapture",
        "python3 -m pytest -q",
        "npm test -- --watch=false",
        "go test ./...",
        "make test",
        "./gradlew test",
        "dotnet test",
    ] {
        let messages = vec![
            ChatMessage::text("user", "fix the tests"),
            shell("call-1", command),
            tool(r#"{"exit_code":1,"stdout":"1 failed"}"#),
        ];
        assert_eq!(
            effort(&messages, false, false),
            ReasoningEffort::Low,
            "{command} is a test run, not a diagnostic surprise"
        );
    }
}

/// A command that also builds is not a pure test run: the failure may be the
/// compiler's, and the model cannot tell which segment exited non-zero.
#[test]
fn a_mixed_build_and_test_command_raises_the_effort() {
    let messages = vec![
        ChatMessage::text("user", "make it pass"),
        shell("call-1", "cargo build && cargo test"),
        tool(r#"{"exit_code":101,"stderr":"error[E0308]"}"#),
    ];
    assert_eq!(effort(&messages, false, false), ReasoningEffort::High);
    assert!(diagnostic(&messages, false));
}

/// The first diagnostic failure earns the expensive request. A second one in
/// a row means the first diagnosis did not settle it, so the next request
/// thinks at the middle tier rather than paying high again.
#[test]
fn a_second_consecutive_diagnostic_failure_thinks_medium() {
    let messages = vec![
        ChatMessage::text("user", "fix it"),
        shell("call-1", "cargo build"),
        tool(r#"{"exit_code":101,"stderr":"error[E0308]"}"#),
    ];
    assert_eq!(effort(&messages, false, true), ReasoningEffort::Medium);
    assert!(diagnostic(&messages, true));
}

/// An expected red test suite between two diagnostics is not a diagnostic
/// failure, so it breaks the streak: the next real diagnostic is the first.
#[test]
fn an_expected_test_failure_breaks_the_diagnostic_streak() {
    let messages = vec![
        ChatMessage::text("user", "fix it"),
        shell("call-1", "cargo test"),
        tool(r#"{"exit_code":1,"stdout":"test result: FAILED"}"#),
    ];
    assert_eq!(effort(&messages, false, true), ReasoningEffort::Low);
    assert!(!diagnostic(&messages, true));
}

#[test]
fn a_timed_out_shell_raises_the_effort() {
    let messages = vec![
        ChatMessage::text("user", "build it"),
        assistant(),
        tool(r#"{"exit_code":0,"timed_out":true}"#),
    ];
    assert_eq!(effort(&messages, false, false), ReasoningEffort::High);
}

#[test]
fn non_empty_post_edit_diagnostics_raise_the_effort() {
    let messages = vec![
        ChatMessage::text("user", "fix it"),
        assistant(),
        tool(r#"{"post_edit_diagnostics":{"diagnostics":[{"message":"unused import"}]}}"#),
    ];
    assert_eq!(effort(&messages, false, false), ReasoningEffort::High);
}

#[test]
fn empty_post_edit_diagnostics_stay_light() {
    let messages = vec![
        ChatMessage::text("user", "fix it"),
        assistant(),
        tool(r#"{"post_edit_diagnostics":{"diagnostics":[]}}"#),
    ];
    assert_eq!(effort(&messages, false, false), ReasoningEffort::Low);
}

/// A batch can mix a test run and a build. One diagnostic result in the batch
/// is enough: the build's failure is the one the model must diagnose.
#[test]
fn one_diagnostic_result_in_a_batch_raises_the_effort() {
    let messages = vec![
        ChatMessage::text("user", "make it pass"),
        ChatMessage {
            role: "assistant".into(),
            content: String::new(),
            tool_calls: vec![
                ToolCall {
                    id: "call-1".into(),
                    name: "shell".into(),
                    arguments: serde_json::json!({ "command": "cargo test" }),
                },
                ToolCall {
                    id: "call-2".into(),
                    name: "shell".into(),
                    arguments: serde_json::json!({ "command": "cargo build" }),
                },
            ],
            reasoning: None,
            reasoning_signature: None,
            ..Default::default()
        },
        tool_result(
            "call-1",
            r#"{"exit_code":1,"stdout":"test result: FAILED"}"#,
        ),
        tool_result("call-2", r#"{"exit_code":101,"stderr":"error[E0308]"}"#),
    ];
    assert_eq!(effort(&messages, false, false), ReasoningEffort::High);
    assert!(diagnostic(&messages, false));
}

#[test]
fn only_the_results_immediately_preceding_the_request_count() {
    let messages = vec![
        ChatMessage::text("user", "find the answer"),
        assistant(),
        tool(r#"{"error":"old failure"}"#),
        ChatMessage::text("user", "resume"),
    ];
    assert_eq!(
        effort(&messages, false, true),
        ReasoningEffort::Low,
        "a failure behind a later user turn is not this request's failure"
    );
    assert!(!diagnostic(&messages, true));
}

#[path = "reasoning_policy_diagnostic_tests.rs"]
mod diagnostic;

#[test]
fn a_configured_effort_pins_every_request() {
    let clean = vec![ChatMessage::text("user", "hi")];
    for configured in [
        ReasoningEffort::Low,
        ReasoningEffort::Medium,
        ReasoningEffort::High,
    ] {
        assert_eq!(
            decide(&clean, true, false, Some(configured)).effort,
            configured,
            "the first request must not override a pinned effort"
        );
        assert_eq!(
            decide(&clean, false, false, Some(configured)).effort,
            configured,
            "a later request must not override a pinned effort"
        );
    }
}
