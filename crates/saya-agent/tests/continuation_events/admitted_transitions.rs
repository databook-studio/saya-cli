use super::*;
use saya_agent::{
    AgentError, AgentLimits, AgentMetricCount, AllowReadOnlyApproval, run_agent_with_sink,
};

/// (a) An output cap that is admitted as a continuation and then answers
/// naturally: exactly one `Continuing { index: 1, limit: 3 }` on the real
/// stream, and the run is NOT truncated — the transition is visible although
/// `truncated` is false.
#[tokio::test]
async fn admitted_continuation_emits_one_continuing_and_the_run_finishes_untruncated() {
    let (provider, requests) = continuation_script(false);
    let sink = RecordingSink::default();
    let output = run_agent_with_sink(
        &provider,
        &NoTools,
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &sink,
        CancellationToken::new(),
    )
    .await
    .expect("the run recovers after one truncation");
    assert_eq!(
        requests.lock().unwrap().len(),
        2,
        "one truncated turn plus one cooperative turn"
    );
    assert!(
        !output.truncated,
        "the natural answer must not mark the run truncated"
    );
    assert_eq!(
        sink.continuation_events(),
        vec![(ProviderContinuationPhase::Continuing, 1, 3)],
        "exactly one Continuing at the admitted continuation: {:?}",
        output.events
    );
    assert_eq!(
        sink.metric_count(&AgentMetric::Count(AgentMetricCount::AdmittedContinuation)),
        1,
        "the event shares the admitted-continuation point"
    );
}

/// (b) With the allowance spent the run still returns the original truncation
/// error, and the stream carries one `Exhausted { index: 3, limit: 3 }` after
/// exactly the three admitted `Continuing` events — never a fourth `Continuing`.
#[tokio::test]
async fn spent_allowance_emits_exhausted_after_exactly_three_continuings() {
    let (provider, requests) = continuation_script(true);
    let sink = RecordingSink::default();
    let error = run_agent_with_sink(
        &provider,
        &NoTools,
        request(),
        definitions(),
        AgentLimits {
            max_continuations: Some(3),
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
        4,
        "one initial turn plus three continuations"
    );
    assert!(
        matches!(
            error,
            AgentError::Provider(ProviderError::OutputTruncated { .. })
        ),
        "the original truncation error still surfaces: {error:?}"
    );
    assert_eq!(
        sink.continuation_events(),
        vec![
            (ProviderContinuationPhase::Continuing, 1, 3),
            (ProviderContinuationPhase::Continuing, 2, 3),
            (ProviderContinuationPhase::Continuing, 3, 3),
            (ProviderContinuationPhase::Exhausted, 3, 3),
        ],
        "the spent allowance ends the run with one Exhausted: {:?}",
        sink.events.lock().unwrap()
    );
}
