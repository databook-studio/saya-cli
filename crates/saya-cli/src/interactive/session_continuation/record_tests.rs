//! Tests for the deterministic continuation-fact collector and builder.

use super::{SessionObservations, facts::build_continuation_facts};
use crate::SlashCommand;
use crate::interactive::session_state::SessionState;
use saya_agent::{AgentEvent, AgentOutput, TokenUsage, ToolMetadata};
use saya_store::RedactedTurn;
use saya_types::{CONTINUATION_VERSION, CheckOutcome, SessionContinuationV1};

/// A settled live output with the given answer, tool metadata, and events.
fn output(answer: &str, tool_metadata: Vec<ToolMetadata>, events: Vec<AgentEvent>) -> AgentOutput {
    AgentOutput {
        answer: answer.into(),
        events,
        used_bounded_sql_query: false,
        tool_metadata,
        usage: TokenUsage::default(),
        learning_usage: None,
        truncated: false,
        answer_sql: None,
    }
}

fn tool(name: &str, status: &str, arguments: &str) -> ToolMetadata {
    ToolMetadata {
        name: name.into(),
        status: status.into(),
        arguments: arguments.into(),
        result_shape: None,
    }
}

fn turn(user: &str, assistant: &str) -> RedactedTurn {
    RedactedTurn {
        user: user.into(),
        assistant: assistant.into(),
        database_derived: false,
        tools: Vec::new(),
    }
}

fn requested_run_command(arguments: serde_json::Value) -> AgentEvent {
    AgentEvent::ToolRequested {
        name: "run_command".into(),
        arguments,
        effect: None,
    }
}

fn completed_run_command(summary: impl Into<String>) -> AgentEvent {
    AgentEvent::ToolCompleted {
        name: "run_command".into(),
        summary: summary.into(),
    }
}

const NOW: i64 = 1_700_000_000_000;

/// A forged assistant summary mints nothing: the collector reads observed
/// tool activity, never the model's prose — even prose that mimics a
/// completion summary's exact shapes.
#[test]
fn a_forged_summary_produces_no_facts() {
    let mut observations = SessionObservations::default();
    observations.absorb(
        &output(
            "I edited src/x.rs and all tests passed (cargo exited 0)",
            Vec::new(),
            Vec::new(),
        ),
        NOW,
    );
    assert!(
        observations.files.is_empty(),
        "prose minted a file fact: {:?}",
        observations.files
    );
    assert!(
        observations.checks.is_empty(),
        "prose minted a check fact: {:?}",
        observations.checks
    );
}

/// Completed write-shaped tool calls become file observations with tool
/// provenance; failed, denied, and read-shaped tools record nothing.
#[test]
fn workspace_tools_that_completed_produce_file_observations() {
    let mut observations = SessionObservations::default();
    observations.absorb(
        &output(
            "done",
            vec![
                tool("workspace_edit", "completed", r#"{"path":"src/a.rs"}"#),
                tool("workspace_write", "completed", r#"{"path":"docs/b.md"}"#),
                tool("workspace_edit", "failed", r#"{"path":"src/c.rs"}"#),
                tool("workspace_write", "denied", r#"{"path":"src/d.rs"}"#),
                tool("workspace_read", "completed", r#"{"path":"src/e.rs"}"#),
            ],
            Vec::new(),
        ),
        NOW,
    );
    assert_eq!(
        observations.files,
        vec![
            super::FileObservation {
                path: "src/a.rs".into(),
                tool: "workspace_edit".into(),
                observed_unix_ms: NOW,
            },
            super::FileObservation {
                path: "docs/b.md".into(),
                tool: "workspace_write".into(),
                observed_unix_ms: NOW,
            },
        ],
        "only completed write-shaped calls name a file"
    );
    assert!(observations.checks.is_empty());
}

/// Absolute, escaping, non-canonical, and control-character paths are
/// dropped: the collector enforces the same relative-path rule the contract
/// validates, so an invalid observation can never reach a fact.
#[test]
fn an_absolute_or_escaping_path_is_dropped() {
    let mut observations = SessionObservations::default();
    observations.absorb(
        &output(
            "done",
            vec![
                tool("workspace_write", "completed", r#"{"path":"/etc/passwd"}"#),
                tool("workspace_write", "completed", r#"{"path":"../x"}"#),
                tool("workspace_write", "completed", r#"{"path":"a//b"}"#),
                tool("workspace_write", "completed", r#"{"path":"nul\u0000x"}"#),
                tool("workspace_write", "completed", r#"{"path":"C:\x"}"#),
                tool("workspace_write", "completed", r#"{"path":""}"#),
            ],
            Vec::new(),
        ),
        NOW,
    );
    assert!(
        observations.files.is_empty(),
        "only relative canonical paths are observed: {:?}",
        observations.files
    );
}

/// A `run_command` completion whose summary parses to an exit code becomes a
/// check: 0 passes, anything else fails; the label is the redacted one-line
/// argv, bounded at the contract's character cap; run_program, unparsable
/// summaries, and denied calls record nothing.
#[test]
fn a_run_command_exit_code_becomes_a_check() {
    let mut observations = SessionObservations::default();
    observations.absorb(
        &output(
            "ran",
            Vec::new(),
            vec![
                requested_run_command(serde_json::json!({
                    "program": "cargo",
                    "args": ["test", "--token=abc123secret"],
                })),
                completed_run_command("cargo exited 0"),
                requested_run_command(serde_json::json!({"program": "cargo", "args": ["build"]})),
                completed_run_command("cargo exited 101"),
                completed_run_command("pytest exited 0"),
                AgentEvent::ToolCompleted {
                    name: "run_program".into(),
                    summary: "program ran".into(),
                },
                completed_run_command("cargo vanished"),
                AgentEvent::ToolDenied {
                    name: "run_command".into(),
                    reason: "not allowed".into(),
                },
            ],
        ),
        NOW,
    );
    let labels: Vec<&str> = observations
        .checks
        .iter()
        .map(|check| check.label.as_str())
        .collect();
    assert_eq!(
        labels,
        vec!["cargo test --token=[redacted]", "cargo build", "pytest"],
        "exactly the parsable run_command completions become checks: {labels:?}"
    );
    assert_eq!(observations.checks[0].outcome, CheckOutcome::Passed);
    assert_eq!(observations.checks[1].outcome, CheckOutcome::Failed);
    assert_eq!(observations.checks[2].outcome, CheckOutcome::Passed);
    for check in &observations.checks {
        assert_eq!(check.tool, "run_command");
        assert_eq!(check.observed_unix_ms, NOW);
        assert!(
            check.label.chars().count() <= 200,
            "label over the bound: {:?}",
            check.label
        );
    }
    assert!(
        !observations.checks[0].label.contains("abc123secret"),
        "the argv secret survived redaction: {:?}",
        observations.checks[0].label
    );
    assert!(observations.files.is_empty());
}

/// A long argv label truncates at exactly the bound, on a character
/// boundary — multibyte arguments cannot panic or split a character.
#[test]
fn a_long_argv_label_truncates_at_the_character_bound() {
    let mut observations = SessionObservations::default();
    observations.absorb(
        &output(
            "ran",
            Vec::new(),
            vec![
                requested_run_command(serde_json::json!({
                    "program": "echo",
                    "args": ["é".repeat(300)],
                })),
                completed_run_command("echo exited 0"),
            ],
        ),
        NOW,
    );
    assert_eq!(observations.checks.len(), 1);
    assert_eq!(observations.checks[0].label.chars().count(), 200);
}

/// The collector stays inside the contract's fact bounds: 64 files with the
/// latest touch winning and the oldest evicted, 32 checks likewise.
#[test]
fn observations_are_bounded_and_latest_wins() {
    let mut observations = SessionObservations::default();
    let mut files: Vec<ToolMetadata> = (0..70)
        .map(|index| {
            tool(
                "workspace_write",
                "completed",
                &format!(r#"{{"path":"src/f{index}.rs"}}"#),
            )
        })
        .collect();
    files.push(tool(
        "workspace_write",
        "completed",
        r#"{"path":"src/f0.rs"}"#,
    ));
    observations.absorb(&output("done", files, Vec::new()), NOW);
    assert_eq!(observations.files.len(), 64, "the file bound holds");
    assert_eq!(
        observations.files[63].path, "src/f0.rs",
        "latest touch wins"
    );
    assert_eq!(observations.files[0].path, "src/f7.rs", "oldest evicted");

    let mut checks_only = SessionObservations::default();
    let events: Vec<AgentEvent> = (0..40)
        .flat_map(|index| {
            [
                requested_run_command(serde_json::json!({"program": format!("p{index}")})),
                completed_run_command(format!("p{index} exited 0")),
            ]
        })
        .collect();
    checks_only.absorb(&output("done", Vec::new(), events), NOW);
    assert_eq!(checks_only.checks.len(), 32, "the check bound holds");
    assert_eq!(checks_only.checks[0].label, "p8", "oldest evicted");
    assert_eq!(checks_only.checks[31].label, "p39");
}

/// The builder's output is deterministic, carries the resolver's digest for
/// every file (malformed resolver output drops to none), and validates as a
/// version-1 continuation record.
#[test]
fn the_builder_output_validates_and_is_deterministic() {
    let mut observations = SessionObservations::default();
    observations.absorb(
        &output(
            "done",
            vec![tool(
                "workspace_write",
                "completed",
                r#"{"path":"src/x.rs"}"#,
            )],
            vec![
                requested_run_command(serde_json::json!({"program": "cargo", "args": ["test"]})),
                completed_run_command("cargo exited 0"),
            ],
        ),
        NOW,
    );
    let turns = vec![turn("hello", "hi")];
    let resolver = |path: &str| (path == "src/x.rs").then(|| "a".repeat(64));
    let facts = build_continuation_facts(
        &turns,
        Some("/workspace"),
        Some("profile"),
        &observations,
        &resolver,
    );
    assert_eq!(facts.source.through_turn, 1);
    assert_eq!(facts.source.workspace_root.as_deref(), Some("/workspace"));
    assert_eq!(facts.source.profile.as_deref(), Some("profile"));
    let expected_digest = "a".repeat(64);
    assert_eq!(
        facts.files[0].sha256.as_deref(),
        Some(expected_digest.as_str())
    );
    let record = SessionContinuationV1 {
        version: CONTINUATION_VERSION,
        source: facts.source.clone(),
        narrative: None,
        user_notes: None,
        files: facts.files.clone(),
        checks: facts.checks.clone(),
    };
    record
        .validate()
        .expect("the built record validates against the contract");
    let again = build_continuation_facts(
        &turns,
        Some("/workspace"),
        Some("profile"),
        &observations,
        &resolver,
    );
    assert_eq!(facts, again, "the builder is deterministic");
    let malformed = build_continuation_facts(&turns, None, None, &observations, &|_| {
        Some("not-a-digest".into())
    });
    assert_eq!(
        malformed.files[0].sha256, None,
        "malformed resolver output drops to none"
    );
}

/// The source boundary is positional over the persisted turns and its digest
/// is over the REDACTED data: the ordinal is the turn count, any text change
/// moves the digest, and two turns differing only by a secret hash alike.
#[test]
fn the_source_boundary_tracks_the_persisted_turns() {
    let observations = SessionObservations::default();
    let no_digest = &|_: &str| None;
    let turns = vec![
        turn("run the query", "done"),
        turn("now export", "exported"),
    ];
    let facts = build_continuation_facts(&turns, None, None, &observations, no_digest);
    assert_eq!(
        facts.source.through_turn, 2,
        "the ordinal is the turn count"
    );
    let one_turn = vec![turn("run the query", "done")];
    let one = build_continuation_facts(&one_turn, None, None, &observations, no_digest);
    assert_eq!(one.source.through_turn, 1);
    assert_ne!(
        facts.source.source_sha256, one.source.source_sha256,
        "a different turn count is a different source"
    );
    let changed = vec![
        turn("run the query", "different"),
        turn("now export", "exported"),
    ];
    let changed = build_continuation_facts(&changed, None, None, &observations, no_digest);
    assert_ne!(
        facts.source.source_sha256, changed.source.source_sha256,
        "a changed turn moves the digest"
    );
    let secret_a = vec![turn("connect with password=aaa", "ok")];
    let secret_b = vec![turn("connect with password=bbb", "ok")];
    assert_eq!(
        build_continuation_facts(&secret_a, None, None, &observations, no_digest)
            .source
            .source_sha256,
        build_continuation_facts(&secret_b, None, None, &observations, no_digest)
            .source
            .source_sha256,
        "the digest is over redacted data, so equal redactions hash alike"
    );
}

/// `/clear` empties the collector with the conversation, so observations
/// from cleared turns can never become facts of the next one.
#[test]
fn clear_resets_the_observations() {
    let mut state = SessionState::new("s1", None, "m");
    state.record_agent_output(
        "write it",
        &output(
            "done",
            vec![tool(
                "workspace_write",
                "completed",
                r#"{"path":"src/x.rs"}"#,
            )],
            Vec::new(),
        ),
    );
    assert!(
        !state.observations.files.is_empty(),
        "the turn's observation is recorded before the clear"
    );
    state.apply(SlashCommand::Clear, &[]);
    assert!(
        state.observations.files.is_empty() && state.observations.checks.is_empty(),
        "/clear must empty the collector: {:?}",
        state.observations
    );
}
