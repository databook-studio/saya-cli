use super::workspace_retry_support::*;
use saya_agent::{
    AgentError, AgentRunControl, AllowReadOnlyApproval, AnsweringLimits, CancellationToken,
    EffectOutcome, NoopEventSink, run_agent_with_control,
};

/// A failed mutation settles its effect exactly as before, and the read that
/// follows is executed like any other call: the settlement does not gate a
/// retry, because nothing does.
#[tokio::test]
async fn a_failed_mutation_still_settles_and_the_read_is_retried() {
    for (failure, expected) in [
        ("write_unknown", EffectOutcome::Unknown),
        ("write_refused", EffectOutcome::Succeeded),
    ] {
        let files = Files::new(failure);
        let provider = Script::new(vec![
            turn(vec![call("first", "read", "a.txt")]),
            turn(vec![call("mutation", "write", "a.txt")]),
            turn(vec![call("retry", "read", "a.txt")]),
            done(),
        ]);
        let owner = Owner::default();
        let control = AgentRunControl::new(
            limits(),
            AnsweringLimits::default(),
            &AllowReadOnlyApproval,
            &NoopEventSink,
            CancellationToken::new(),
        )
        .with_effect_admission(&owner);
        run_agent_with_control(&provider, &files, request(), definitions(false), control)
            .await
            .unwrap();
        assert_eq!(files.read_count("a.txt"), 2);
        assert_eq!(
            *owner.tools.lock().unwrap(),
            vec![EffectOutcome::Succeeded, expected, EffectOutcome::Succeeded],
            "the retried read is admitted like any other call"
        );
    }
}

#[tokio::test]
async fn a_failed_authoritative_write_settlement_stops_before_retry() {
    let files = Files::new("typed");
    let provider = Script::new(vec![
        turn(vec![call("first", "read", "a.txt")]),
        turn(vec![call("mutation", "write", "a.txt")]),
        turn(vec![call("retry", "read", "a.txt")]),
        done(),
    ]);
    let mut owner = Owner::default();
    owner.fail_write_settlement = true;
    let cancellation = CancellationToken::new();
    let control = AgentRunControl::new(
        limits(),
        AnsweringLimits::default(),
        &AllowReadOnlyApproval,
        &NoopEventSink,
        cancellation.clone(),
    )
    .with_effect_admission(&owner);
    let result =
        run_agent_with_control(&provider, &files, request(), definitions(false), control).await;
    assert!(matches!(result, Err(AgentError::EffectOwnerFailure(_))));
    assert!(cancellation.is_cancelled());
    assert_eq!(files.read_count("a.txt"), 1);
    assert_eq!(
        std::fs::read(files.root.join("a.txt")).unwrap(),
        b"created\n"
    );
    assert_eq!(owner.tools.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn cancellation_after_a_real_write_stops_before_any_retry() {
    let files = Files::new("cancel_write");
    let provider = Script::new(vec![
        turn(vec![call("first", "read", "a.txt")]),
        turn(vec![call("mutation", "write", "a.txt")]),
        turn(vec![call("retry", "read", "a.txt")]),
        done(),
    ]);
    let owner = Owner::default();
    let control = AgentRunControl::new(
        limits(),
        AnsweringLimits::default(),
        &AllowReadOnlyApproval,
        &NoopEventSink,
        CancellationToken::new(),
    )
    .with_effect_admission(&owner);
    assert!(matches!(
        run_agent_with_control(&provider, &files, request(), definitions(false), control).await,
        Err(AgentError::Cancelled)
    ));
    assert_eq!(files.read_count("a.txt"), 1);
    assert_eq!(
        std::fs::read(files.root.join("a.txt")).unwrap(),
        b"created\n"
    );
    assert_eq!(
        *owner.tools.lock().unwrap(),
        vec![EffectOutcome::Succeeded, EffectOutcome::Succeeded]
    );
}
