use super::*;

/// The prose settlement maps to the terminal status, a latched budget stop
/// reclassifying Failed/Cancelled.
#[test]
fn prose_status_maps_the_settlement_and_the_latched_budget() {
    assert_eq!(
        prose_status(ProseOutcome::Completed, false),
        TurnStatus::Completed
    );
    assert_eq!(
        prose_status(ProseOutcome::Completed, true),
        TurnStatus::Completed
    );
    assert_eq!(
        prose_status(ProseOutcome::Failed, false),
        TurnStatus::Failed
    );
    assert_eq!(
        prose_status(ProseOutcome::Failed, true),
        TurnStatus::BudgetStopped
    );
    assert_eq!(
        prose_status(ProseOutcome::Cancelled, false),
        TurnStatus::Cancelled
    );
    assert_eq!(
        prose_status(ProseOutcome::Cancelled, true),
        TurnStatus::BudgetStopped
    );
}

/// Exit code 0 belongs to a completed turn only.
#[test]
fn exit_code_is_zero_only_for_a_completed_turn() {
    assert_eq!(exit_code(TurnStatus::Completed), 0);
    assert_eq!(exit_code(TurnStatus::Failed), 5);
    assert_eq!(exit_code(TurnStatus::Cancelled), 5);
    assert_eq!(exit_code(TurnStatus::BudgetStopped), 5);
}

/// `final_summary` enforces the one invariant the scripted-run contract
/// promises: the rendered `exit_code` equals the process exit code for
/// every (settled status × run outcome) combination.
#[test]
fn final_summary_enforces_the_exit_code_invariant_for_every_combination() {
    let statuses = [
        TurnStatus::Completed,
        TurnStatus::Failed,
        TurnStatus::Cancelled,
        TurnStatus::BudgetStopped,
    ];
    for status in statuses {
        for errored in [true, false] {
            let summary = final_summary(
                TurnSummary {
                    status,
                    tally: ToolTally::default(),
                },
                errored,
            );
            let process_code = if errored { 5 } else { 0 };
            assert_eq!(
                exit_code(summary.status),
                process_code,
                "{status:?} with the run errored={errored} must render the process code"
            );
        }
    }
}

/// A completed settlement whose run ended errored — the refused per-turn
/// save at the record cap — reclassifies as `failed` (exit 5); a failed,
/// cancelled, or budget-stopped settlement keeps its status when the run
/// errored; a run that did not error reports completed.
#[test]
fn final_summary_maps_the_outcome_over_the_settled_status() {
    let summary = |status| TurnSummary {
        status,
        tally: ToolTally::default(),
    };
    assert_eq!(
        final_summary(summary(TurnStatus::Completed), true).status,
        TurnStatus::Failed,
        "a completed turn whose save was refused ends failed, never completed/0"
    );
    assert_eq!(
        final_summary(summary(TurnStatus::Failed), true).status,
        TurnStatus::Failed
    );
    assert_eq!(
        final_summary(summary(TurnStatus::Cancelled), true).status,
        TurnStatus::Cancelled
    );
    assert_eq!(
        final_summary(summary(TurnStatus::BudgetStopped), true).status,
        TurnStatus::BudgetStopped
    );
    for status in [
        TurnStatus::Completed,
        TurnStatus::Failed,
        TurnStatus::Cancelled,
        TurnStatus::BudgetStopped,
    ] {
        assert_eq!(
            final_summary(summary(status), false).status,
            TurnStatus::Completed,
            "a run that did not error reports completed: {status:?}"
        );
    }
}

/// The reclassification never disturbs the tool tally: the refused-save
/// run still reports the calls that completed and the ones whose effects
/// are unknown.
#[test]
fn final_summary_keeps_the_tally_through_the_reclassification() {
    let mut tally = ToolTally::default();
    tally.record(&AgentEvent::ToolRequested {
        name: "workspace_edit".to_owned(),
        arguments: serde_json::json!({"path": "src/lib.rs"}),
        effect: None,
    });
    let summary = TurnSummary {
        status: TurnStatus::Completed,
        tally,
    };
    let settled = final_summary(summary, true);
    assert_eq!(settled.status, TurnStatus::Failed);
    assert_eq!(settled.tally.completed(), 0);
    assert_eq!(
        settled.tally.unfinished_entries(),
        vec!["workspace_edit src/lib.rs"]
    );
}
