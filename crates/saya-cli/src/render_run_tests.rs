//! `render_run.rs` tests: the event-line shaper and the show stanza. The
//! per-endpoint usage view has its own tests in `render_usage.rs`.

use super::*;

/// Json and Ndjson are one serde path with two framings: the bytes are
/// the journal's own line, and the only difference between the two
/// formats is nothing — the framing is the newline either way.
#[test]
fn json_and_ndjson_share_one_serde_path() {
    let event = RunEvent::Paused {
        reason: PauseReason::BudgetExhausted,
    };
    let json = render_run_event(&event, RenderFormat::Json);
    let ndjson = render_run_event(&event, RenderFormat::Ndjson);
    assert_eq!(
        json.stdout, ndjson.stdout,
        "one serialization, two framings"
    );
    assert_eq!(ndjson.stdout, format!("{}\n", journal_line(&event)));
    assert!(ndjson.stdout.contains(r#""type":"paused""#), "{ndjson:?}");
    assert!(ndjson.stdout.contains("budget_exhausted"), "{ndjson:?}");
}

/// The lifecycle lines are distinct and say what happened.
#[test]
fn lifecycle_lines_render_distinctly() {
    let lines = [
        RunEvent::RunStarted,
        RunEvent::PlanApproved { scopes: None },
        RunEvent::StepStarted { step: 0 },
        RunEvent::StepCompleted { step: 0 },
        RunEvent::StepFailed { step: 0 },
        RunEvent::Paused {
            reason: PauseReason::WallClockExceeded,
        },
        RunEvent::Completed,
        RunEvent::Failed {
            code: RunFailureCode::SafetyQuery,
        },
        RunEvent::Cancelled,
    ]
    .map(|event| run_event_text(&event));
    for (index, line) in lines.iter().enumerate() {
        assert!(line.ends_with('\n'), "line {index} is newline-terminated");
    }
    for a in 0..lines.len() {
        for b in (a + 1)..lines.len() {
            assert_ne!(lines[a], lines[b], "lines {a} and {b} must differ");
        }
    }
    assert_eq!(lines[2], "step 1 started\n", "steps render one-based");
    assert!(
        lines[5].contains("the wall-clock budget tripped"),
        "{lines:?}"
    );
}

/// An unreported usage figure renders `unknown`, never zero — a provider
/// that reported nothing must not be read as having cost nothing.
#[test]
fn unreported_usage_renders_unknown_not_zero() {
    let event = RunEvent::Usage {
        endpoint: "primary".into(),
        tokens: Some(12),
        turns: None,
        tool_calls: None,
        cached_input_tokens: None,
        cache_creation_input_tokens: None,
    };
    let text = run_event_text(&event);
    assert!(text.contains("tokens 12"), "{text}");
    assert!(text.contains("turns unknown"), "{text}");
    assert!(text.contains("tool calls unknown"), "{text}");
    assert!(!text.contains("0"), "no zero is invented: {text}");
}

/// The download level's text line names the wallet's term — claimed, not
/// downloaded-written: claims are never refunded, so the figure is the
/// budget's own arithmetic — and states it as the running level it is.
#[test]
fn the_download_level_line_states_the_running_spend() {
    let text = run_event_text(&RunEvent::DownloadedBytes { bytes: 97 });
    assert!(text.contains("97"), "{text}");
    assert!(text.ends_with('\n'), "line is newline-terminated");
}

/// A run whose journal holds no usage event renders no usage section —
/// nothing reported is not the same claim as costing nothing.
#[test]
fn the_show_stanza_omits_the_usage_section_when_the_run_recorded_none() {
    let bare = run_show_text(RunShowStanza {
        id: "r-3",
        status: "completed",
        failure_cause: None,
        created_unix_ms: 1,
        updated_unix_ms: 2,
        spec: None,
        paused: None,
        deliverables: &[],
        usage: &[],
    });
    assert!(!bare.contains("usage"), "no section without usage: {bare}");
}

/// The show stanza carries the failure cause and the pause reason in the
/// exact shapes the read surface has always printed.
#[test]
fn the_show_stanza_names_cause_and_pause() {
    let text = run_show_text(RunShowStanza {
        id: "r-1",
        status: "failed",
        failure_cause: Some(failure_code_cause(RunFailureCode::SafetyQuery)),
        created_unix_ms: 100,
        updated_unix_ms: 200,
        spec: Some(("the goal", "workspace-write")),
        paused: Some(PauseReason::StoreUnavailable),
        deliverables: &[],
        usage: &[],
    });
    assert_eq!(
        text,
        "run r-1\nstatus: failed (the safety gate refused a query)\ncreated: 100\n\
         updated: 200\ngoal: the goal\nscopes: workspace-write\n\
         paused: the state store became unavailable"
    );
}

/// Deliverables render inside the shared stanza rather than at a call
/// site, which is what keeps `/runs` and `saya run show` byte-identical.
/// A run that declared none renders no section at all — an empty
/// "deliverables:" heading would read as "it produced nothing", which is
/// a different claim from "it declared nothing".
#[test]
fn the_show_stanza_carries_deliverables_and_omits_the_section_when_there_are_none() {
    let bare = run_show_text(RunShowStanza {
        id: "r-2",
        status: "completed",
        failure_cause: None,
        created_unix_ms: 1,
        updated_unix_ms: 2,
        spec: None,
        paused: None,
        deliverables: &[],
        usage: &[],
    });
    assert!(
        !bare.contains("deliverables"),
        "no section without deliverables: {bare}"
    );
    let listed = run_show_text(RunShowStanza {
        id: "r-2",
        status: "completed",
        failure_cause: None,
        created_unix_ms: 1,
        updated_unix_ms: 2,
        spec: None,
        paused: None,
        deliverables: &["step 0: report.md (29 bytes, sha256 4634)".to_string()],
        usage: &[],
    });
    assert!(
        listed.ends_with("\ndeliverables:\nstep 0: report.md (29 bytes, sha256 4634)"),
        "deliverables close the stanza: {listed}"
    );
}
