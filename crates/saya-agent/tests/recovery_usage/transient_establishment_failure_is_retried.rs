/// A transient transport failure before any stream existed is re-sent under the
/// turn's own schedule, not settled as a refused request. This is the chat and
/// `ask` path: both drive the same receive loop the job does.
#[tokio::test]
async fn a_transient_establishment_failure_is_retried() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent_with_sink(
        &UnavailableThenSuccess {
            attempts: Mutex::new(0),
        },
        &NoTools {
            calls: Arc::new(Mutex::new(0)),
        },
        request(),
        Vec::new(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &RecordingSink(events.clone()),
        CancellationToken::new(),
    )
    .await
    .expect("the retry answers");
    assert_eq!(output.answer, "recovered");
    let seen = events.lock().unwrap();
    assert!(
        seen.iter().any(|event| matches!(
            event,
            AgentEvent::ProviderRecovery {
                phase: ProviderRecoveryPhase::Retrying,
                reason: ProviderRecoveryReason::ProviderFailure,
                attempt: 2,
                limit: 9,
            }
        )),
        "the transient failure is retried on attempt 2: {seen:?}"
    );
    assert!(
        !seen.iter().any(|event| matches!(
            event,
            AgentEvent::ProviderRecovery {
                phase: ProviderRecoveryPhase::NotRetried,
                ..
            }
        )),
        "nothing about a transient transport failure is settled as unretried: {seen:?}"
    );
}
