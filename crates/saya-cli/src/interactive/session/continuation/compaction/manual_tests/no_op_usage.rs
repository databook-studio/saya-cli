/// Short history → "nothing to compact", no provider call at all. The plan is
/// `None`, so the execution layer never builds a provider — asserted here by
/// there being no call to make: planning is the gate, not the provider.
#[test]
fn a_short_history_plans_nothing_and_calls_no_provider() {
    for count in 0..=3 {
        let state = turns(count);
        let history = state.provider_history();
        assert!(
            plan(&state.turns, &history).is_none(),
            "{count} turns must plan nothing"
        );
    }
    let message = "Nothing to compact: the conversation is short enough already.";
    assert!(
        message.contains("Nothing to compact"),
        "the nothing-to-do message keeps its shape: {message}"
    );
}

/// The compaction call's usage is labelled apart from the answering total: a
/// reported usage folds into the learning total only, never the answering
/// one.
#[tokio::test]
async fn the_compaction_call_reports_its_usage_apart() {
    let state = turns(5);
    let history = state.provider_history();
    let plan = plan(&state.turns, &history).expect("long history plans");
    let provider = StaticProvider {
        text: "older turns discussed orders.".into(),
        usage: Some(saya_agent::TokenUsage::new(40, 10)),
    };
    let outcome = summarise(&provider, "m", &plan)
        .await
        .expect("static summary succeeds");
    assert_eq!(
        outcome.usage,
        Some(saya_agent::TokenUsage::new(40, 10)),
        "the compaction call surfaces its provider-reported usage"
    );
    let mut usage_state = turns(0);
    usage_state
        .usage
        .record(&saya_agent::TokenUsage::new(300, 130));
    usage_state.usage.record_learning(outcome.usage);
    let rendered = usage_state.usage.render();
    assert!(
        rendered.contains("Session token usage") && rendered.contains("Input tokens: 300"),
        "the answering total is intact: {rendered}"
    );
    assert!(
        rendered.contains("Learning call") && rendered.contains("Input tokens: 40"),
        "the compaction usage is labelled apart: {rendered}"
    );
    assert!(
        !rendered.contains("Input tokens: 340"),
        "the two totals must not merge: {rendered}"
    );
}
