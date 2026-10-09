use super::*;

#[tokio::test]
async fn usage_accumulation_keeps_not_reported_distinct_from_zero() {
    let clock = SimClock::new();
    let root = temp_root("usage");
    let (store, run_id) = seeded_store(&root, "usage", RunStatus::Executing).await;
    fs::create_dir_all(root.join("run")).unwrap();
    let clock_for_sink = clock.clone();
    let sink = EngineEventSink::new(
        run_id,
        RunState::Executing,
        Journal::open(root.join("run")),
        store,
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::default(),
        },
        move || clock_for_sink.clock(),
    );

    // Before any provider call: nothing is known, and nothing is read as
    // zero — the optional figures are `None`, not `Some(0)`.
    assert_eq!(sink.usage(), UsageTotals::default());

    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(100, 30),
        },
    )
    .await;
    let totals = sink.usage();
    assert_eq!(totals.input_tokens, 100);
    assert_eq!(totals.output_tokens, 30);
    assert_eq!(
        totals.cached_input_tokens, None,
        "an unreported figure must stay unknown, never zero"
    );

    // A *reported* zero is a number, not unknown: it survives as `Some(0)`.
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Extraction,
            usage: TokenUsage::new(10, 2).with_cached_input(Some(0)),
        },
    )
    .await;
    assert_eq!(
        sink.usage().cached_input_tokens,
        Some(0),
        "a reported zero is a number, not 'not reported'"
    );

    // Later reports sum in; a call that reports nothing never folds in as a
    // zero for that figure.
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(5, 1)
                .with_cached_input(Some(5))
                .with_reasoning(Some(3)),
        },
    )
    .await;
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(7, 0).with_cache_creation(Some(2)),
        },
    )
    .await;
    let totals = sink.usage();
    assert_eq!(totals.input_tokens, 122);
    assert_eq!(totals.output_tokens, 33);
    assert_eq!(
        totals.cached_input_tokens,
        Some(5),
        "the last call did not report cache reads; they must not fold in as zero"
    );
    assert_eq!(totals.cache_creation_input_tokens, Some(2));
    assert_eq!(totals.reasoning_tokens, Some(3));

    // Usage counting never moves the machine: the run still executes.
    assert_eq!(sink.state(), RunState::Executing);

    let _ = fs::remove_dir_all(root);
}

/// The seeding path's twin of the accumulation test above: a sink seeded
/// from its journal — what a resume does — keeps "not reported" distinct
/// from zero across the seam. The journal carries only the combined
/// input-plus-output figure per call, so the seeded totals hold that spend
/// whole (`carried_tokens`), the optional figures no recorded call reported
/// stay `None`, no split is fabricated, and the invocation's own reports
/// fold in under the same rule afterwards.
#[tokio::test]
async fn seeding_from_the_journal_keeps_not_reported_distinct_from_zero() {
    let root = temp_root("seeded-usage");
    let (store, run_id) = seeded_store(&root, "seeded-usage", RunStatus::Executing).await;
    let run_dir = root.join("run");
    fs::create_dir_all(&run_dir).unwrap();
    let journaled = vec![journaled_usage(100), journaled_usage(60)];
    let journal = Journal::open(&run_dir);
    for event in &journaled {
        journal.append(event).unwrap();
    }
    let sink = EngineEventSink::new(
        run_id,
        RunState::Executing,
        Journal::open(&run_dir),
        store,
        SinkBudgets {
            wall_clock: None,
            token_ceiling: None,
            download_budget: None,
            carried_usage: UsageTotals::from_journal(&journaled),
        },
        Instant::now,
    );

    // The seeded totals: the record's combined spend, every optional figure
    // still unknown — never zero — and no input/output split the journal
    // never held.
    let seeded = sink.usage();
    assert_eq!(seeded.carried_tokens, 160);
    assert_eq!(
        (seeded.input_tokens, seeded.output_tokens),
        (0, 0),
        "the record holds no split, and none is fabricated"
    );
    assert_eq!(
        seeded.cached_input_tokens, None,
        "a figure no recorded call reported stays unknown, never zero"
    );
    assert_eq!(seeded.cache_creation_input_tokens, None);
    assert_eq!(seeded.reasoning_tokens, None, "the journal carries none");

    // A *reported* zero from this invocation is a number, not unknown: it
    // survives as `Some(0)` on the seeded sink too.
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Extraction,
            usage: TokenUsage::new(10, 2).with_cached_input(Some(0)),
        },
    )
    .await;
    assert_eq!(
        sink.usage().cached_input_tokens,
        Some(0),
        "a reported zero is a number, not 'not reported'"
    );

    // Later live reports sum in under the same rule; a call that reports
    // nothing never folds in as a zero, and the carried spend stands apart
    // from the invocation's own figures.
    AgentEventSink::emit(
        &sink,
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage: TokenUsage::new(5, 1)
                .with_cached_input(Some(5))
                .with_reasoning(Some(3)),
        },
    )
    .await;
    let totals = sink.usage();
    assert_eq!(totals.carried_tokens, 160, "seeding is not re-folded");
    assert_eq!(totals.input_tokens, 15);
    assert_eq!(totals.output_tokens, 3);
    assert_eq!(
        totals.cached_input_tokens,
        Some(5),
        "the last call did not report cache reads; they must not fold in as zero"
    );
    assert_eq!(totals.reasoning_tokens, Some(3));

    let _ = fs::remove_dir_all(root);
}
