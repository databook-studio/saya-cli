use super::*;

/// The one that matters: the token ceiling binds the **run**, not each
/// invocation. A run pauses on its 150-token ceiling having spent 160; a
/// resume seeds the sink's totals from the journal's usage record instead of
/// re-arming the ceiling, so — resumed with no fresh budget — the run pauses
/// again at its cumulative spend. The same cancellation owner sees the
/// carried spend before the resumed provider is polled, so there is no new
/// request after the run already crossed its ceiling.
#[tokio::test]
async fn a_budget_paused_run_resumed_without_a_fresh_budget_pauses_at_its_cumulative_spend() {
    // The posture a `BudgetExhausted` pause leaves: the per-call usage the
    // engine journals, then the pause naming the budget.
    let plan = RunPlan::new(vec![step("the only step")]).unwrap();
    let run = crashed_run(
        "budget-resume",
        &[
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
            journaled_usage(100),
            journaled_usage(60),
            RunEvent::Paused {
                reason: PauseReason::BudgetExhausted,
            },
        ],
        RunStatus::Paused,
        plan,
    )
    .await;
    let provider = UsageProvider::default();

    // Resumed against the same ceiling, granted nothing fresh.
    let outcome = resume(&run.run_dir, budgeted_inputs(&run, &provider, Some(150))).await;

    // The carried known spend is checked before the resumed source starts.
    let error = outcome.unwrap_err();
    assert!(matches!(error, ResumeError::Episode { .. }));

    assert_eq!(
        provider.count(),
        0,
        "an exhausted carried budget admits no request"
    );
    // The journal has the original record followed by exactly one settled
    // budget pause; resume never adds a request or replays a receipt.
    assert_eq!(
        run.journal(),
        vec![
            RunEvent::RunStarted,
            RunEvent::PlanApproved { scopes: None },
            RunEvent::StepStarted { step: 0 },
            journaled_usage(100),
            journaled_usage(60),
            RunEvent::Paused {
                reason: PauseReason::BudgetExhausted
            },
            RunEvent::Paused {
                reason: PauseReason::BudgetExhausted
            },
        ],
        "the resume must pause before admitting work at the carried ceiling"
    );
    let events = run.journal();
    // The run's whole spend across both invocations.
    let total: u64 = events
        .iter()
        .map(|event| match event {
            RunEvent::Usage {
                tokens: Some(tokens),
                ..
            } => *tokens,
            _ => 0,
        })
        .sum();
    assert_eq!(
        total, 160,
        "resume must not fabricate another usage receipt"
    );
    // The store mirrors the pause the resume recorded.
    let recorded = run.store.get_run(&run.run_id).await.unwrap().unwrap();
    assert_eq!(
        recorded.status,
        RunStatus::Paused,
        "the run must stand paused, not running on a fresh budget"
    );
    // No resumed call: the carried known spend already exhausted the ceiling.
    assert_eq!(provider.count(), 0);

    let _ = fs::remove_dir_all(&run.root);
}
