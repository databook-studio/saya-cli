use super::*;
use saya_agent::{
    AgentError, AgentLimits, AgentMetricCount, AgentRunControl, AllowReadOnlyApproval,
    AnsweringLimits, BudgetDimension, RequestPreflightConfig, run_agent_with_control,
    run_agent_with_sink,
};
use std::num::NonZeroU64;

/// (c) A continuation the request preflight refuses (the follow_ons fixture:
/// the re-ask no longer fits the known window) never reaches admission, so no
/// `Continuing` is emitted — and no `Exhausted` either: the run stops on the
/// typed context-window budget, not on the continuation limit.
#[tokio::test]
async fn preflight_refused_continuation_emits_no_continuation_transition() {
    let (provider, requests) = continuation_script(false);
    let sink = RecordingSink::default();
    let prompt_request = AgentRequest {
        prompt: "same user question".into(),
        profile_names: vec!["analytics".into()],
        model: "preflight-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    };
    let control = AgentRunControl::new(
        AgentLimits::default(),
        AnsweringLimits::default(),
        &AllowReadOnlyApproval,
        &sink,
        CancellationToken::new(),
    )
    .with_request_preflight(RequestPreflightConfig::new(
        Some(NonZeroU64::new(1024 + 64 + 400).unwrap()),
        NonZeroU64::new(64).unwrap(),
    ));
    let output = run_agent_with_control(&provider, &NoTools, prompt_request, Vec::new(), control)
        .await
        .expect("the preflight stop is a bounded outcome, not an error");
    assert_eq!(
        requests.lock().unwrap().len(),
        1,
        "the re-ask was refused before reaching the provider"
    );
    assert_eq!(
        sink.metric_count(&AgentMetric::Count(AgentMetricCount::OutputTruncated)),
        1,
        "the fixture is live: the first turn really was truncated"
    );
    assert_eq!(
        sink.metric_count(&AgentMetric::Count(AgentMetricCount::AdmittedContinuation)),
        0,
        "a preflight-refused continuation is not admitted"
    );
    assert!(
        sink.continuation_events().is_empty(),
        "no continuation transition may be emitted for a refused re-ask: {:?}",
        sink.events.lock().unwrap()
    );
    assert!(
        output.events.iter().any(|event| matches!(
            event,
            AgentEvent::AnsweringBudgetStopped {
                dimension: BudgetDimension::RequestContextWindow
            }
        )),
        "the run stopped on the context-window budget: {:?}",
        output.events
    );
}

/// (d) A typed stop that owns termination while a continuation is pending —
/// the turn-ceiling salvage (mirroring `max_turns_ceiling_wins_over_continuations`)
/// — emits no `Exhausted`: the run did not stop because the allowance ran out.
#[tokio::test]
async fn turn_ceiling_stop_with_pending_continuation_emits_no_exhausted() {
    let (provider, requests) = continuation_script(true);
    let sink = RecordingSink::default();
    let output = run_agent_with_sink(
        &provider,
        &NoTools,
        request(),
        definitions(),
        AgentLimits {
            max_turns: Some(2),
            ..AgentLimits::default()
        },
        &AllowReadOnlyApproval,
        &sink,
        CancellationToken::new(),
    )
    .await
    .expect("the turn ceiling salvages instead of erroring");
    assert!(
        output.truncated,
        "the run stopped at the turn ceiling through salvage"
    );
    assert_eq!(
        requests.lock().unwrap().len(),
        3,
        "two ceiling turns plus the salvage call"
    );
    assert_eq!(
        sink.continuation_events(),
        vec![(ProviderContinuationPhase::Continuing, 1, 3)],
        "the one admitted continuation is visible and no Exhausted is emitted: {:?}",
        sink.events.lock().unwrap()
    );
}

/// (e) A disabled continuation (`None` and `Some(0)`) surfaces the truncation
/// immediately and emits neither transition: no continuation ever ran, so no
/// allowance was spent — `Exhausted` is reserved for a finite limit that ran
/// out.
#[tokio::test]
async fn disabled_continuation_emits_neither_transition() {
    for max_continuations in [None, Some(0)] {
        let (provider, requests) = continuation_script(true);
        let sink = RecordingSink::default();
        let error = run_agent_with_sink(
            &provider,
            &NoTools,
            request(),
            definitions(),
            AgentLimits {
                max_continuations,
                ..AgentLimits::default()
            },
            &AllowReadOnlyApproval,
            &sink,
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
        assert_eq!(
            requests.lock().unwrap().len(),
            1,
            "max_continuations={max_continuations:?}: the truncation surfaces immediately"
        );
        assert!(
            matches!(
                error,
                AgentError::Provider(ProviderError::OutputTruncated { .. })
            ),
            "max_continuations={max_continuations:?}: the error surfaces: {error:?}"
        );
        assert!(
            sink.continuation_events().is_empty(),
            "max_continuations={max_continuations:?}: neither transition may be emitted: {:?}",
            sink.events.lock().unwrap()
        );
    }
}
