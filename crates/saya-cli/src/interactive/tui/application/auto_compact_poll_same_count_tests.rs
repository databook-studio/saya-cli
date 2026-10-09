use super::*;

fn polling_success_with_same_count_rebuilt_history_is_rejected(automatic: bool) {
    let (mut app, state) = five_turns_app();
    let original_plan = plan(&state.turns, &state.provider_history()).expect("long history plans");
    let summary = "older turns: q0 q1 q2 discussed orders.";
    let mut rebuilt = SessionState::new("s1", None, "m");
    for index in 0..5 {
        rebuilt.record_turn(
            format!("replacement q{index}"),
            format!("replacement a{index}"),
            false,
            Vec::new(),
        );
    }
    let live_history = rebuilt.provider_history();
    let rebuilt_plan = plan(&rebuilt.turns, &live_history).expect("rebuilt history plans");
    assert_eq!(rebuilt_plan.compacted_turns, original_plan.compacted_turns);
    if !automatic {
        rebuilt.auto_compact_failed = true;
    }
    let auto_compact_failed_before = rebuilt.auto_compact_failed;

    let (tx, rx) = std::sync::mpsc::channel();
    app.compact_task = Some(rx);
    app.request.started = Some(std::time::Instant::now());
    app.request.activity = Some("compacting".into());
    tx.send(CompactOutcome {
        message: crate::interactive::session::continuation::compaction::manual::success_message(
            original_plan.compacted_turns,
            summary,
        ),
        failed: false,
        usage: Some(saya_agent::TokenUsage::new(40, 10)),
        automatic,
        summary: Some(summary.into()),
        compacted_turns: original_plan.compacted_turns,
        source_identity: Some(original_plan.source_identity),
    })
    .expect("the worker's outcome sends");

    crate::interactive::compact_task::poll(&mut app, &mut rebuilt);

    assert_eq!(rebuilt.provider_history(), live_history);
    assert!(rebuilt.compaction_summary.is_none());
    if automatic {
        assert!(rebuilt.auto_compact_failed);
    } else {
        assert_eq!(rebuilt.auto_compact_failed, auto_compact_failed_before);
    }
    assert!(app.compact_task.is_none());
    assert!(app.request.started.is_none() && app.request.activity.is_none());
    assert_eq!(rebuilt.usage.learning.turns, 1);
    let failure = if automatic {
        auto_failure_message("the conversation changed while compacting")
    } else {
        crate::interactive::session::continuation::compaction::manual::failure_message(
            "the conversation changed while compacting",
        )
    };
    let success = if automatic {
        auto_success_message(original_plan.compacted_turns, summary)
    } else {
        crate::interactive::session::continuation::compaction::manual::success_message(
            original_plan.compacted_turns,
            summary,
        )
    };
    let blocks = app.transcript.blocks();
    assert!(blocks.iter().any(|block| {
        block.kind == crate::interactive::tui::transcript::BlockKind::Error && block.text == failure
    }));
    assert!(!blocks.iter().any(|block| block.text == success));
}

#[test]
fn polling_manual_success_with_same_count_rebuilt_history_reports_failure() {
    polling_success_with_same_count_rebuilt_history_is_rejected(false);
}

#[test]
fn polling_automatic_success_with_same_count_rebuilt_history_reports_failure() {
    polling_success_with_same_count_rebuilt_history_is_rejected(true);
}
