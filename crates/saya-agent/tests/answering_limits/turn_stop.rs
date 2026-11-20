use super::*;
use saya_agent::{AgentRunControl, AnsweringScope, run_agent_with_control};
use std::time::Duration;

/// A turn control whose question scope arms its own budget: the stop ends as
/// a budget stop, and it belongs to the turn, so the run's own token is
/// untouched and the next turn on the same run still admits requests.
#[tokio::test]
async fn a_budget_stop_ends_the_turn_and_leaves_the_run_alive() {
    let run = CancellationToken::new();
    let sink = RecordingSink::new(run.clone());
    let (provider, requests) = ScriptedProvider::new(vec![turn("must not run", Vec::new())]);
    let scope = AnsweringScope::new(
        answering_limits(Some(0), None, None),
        CancellationToken::new(),
    );

    let stopped_output = run_agent_with_control(
        &provider,
        &RecordingExecutor::default(),
        request(),
        definitions(),
        AgentRunControl::new(
            AgentLimits::default(),
            answering_limits(Some(0), None, None),
            &AllowReadOnlyApproval,
            &sink,
            run.clone(),
        )
        .with_answering_scope(scope),
    )
    .await
    .expect("a spent question budget is a typed partial answer");

    assert!(stopped(&stopped_output, BudgetDimension::LogicalRequests));
    assert!(
        stopped_output.truncated && stopped_output.answer.is_empty(),
        "the stopped turn carries no answer"
    );
    assert!(
        requests.lock().unwrap().is_empty(),
        "the stop lands before the request reaches the provider"
    );
    assert_eq!(
        run.reason(),
        None,
        "a budget stop stops its turn, never the run it belongs to"
    );

    let (next, next_requests) =
        ScriptedProvider::new(vec![turn("the next turn answers", Vec::new())]);
    let next_scope = AnsweringScope::new(
        answering_limits(Some(4), None, None),
        CancellationToken::new(),
    );
    let next_output = run_agent_with_control(
        &next,
        &RecordingExecutor::default(),
        request(),
        definitions(),
        AgentRunControl::new(
            AgentLimits::default(),
            answering_limits(Some(4), None, None),
            &AllowReadOnlyApproval,
            &sink,
            run.clone(),
        )
        .with_answering_scope(next_scope),
    )
    .await
    .expect("the next turn on the same run answers");
    assert_eq!(next_output.answer, "the next turn answers");
    assert_eq!(next_requests.lock().unwrap().len(), 1);
    assert_eq!(run.reason(), None, "the next turn leaves the run running");
}

/// A question scope handed to a turn runs as a child of the run token: the
/// scope's own budget stop latches the scope and leaves the run running, and a
/// stop of the run still stops the answering work holding a request open.
#[tokio::test]
async fn a_supplied_scope_stops_with_the_run_and_never_stops_it() {
    let run = CancellationToken::new();
    let scope = AnsweringScope::new(
        answering_limits(Some(0), None, None),
        CancellationToken::new(),
    );
    let sink = RecordingSink::new(run.clone());
    let (provider, requests) = ScriptedProvider::new(vec![turn("must not run", Vec::new())]);

    let stopped_output = run_agent_with_control(
        &provider,
        &RecordingExecutor::default(),
        request(),
        definitions(),
        AgentRunControl::new(
            AgentLimits::default(),
            answering_limits(Some(4), None, None),
            &AllowReadOnlyApproval,
            &sink,
            run.clone(),
        )
        .with_answering_scope(scope.clone()),
    )
    .await
    .expect("a spent question budget is a typed partial answer");

    assert!(stopped(&stopped_output, BudgetDimension::LogicalRequests));
    assert!(requests.lock().unwrap().is_empty());
    assert_eq!(
        scope.cancellation_token().reason(),
        Some(CancellationReason::Budget(BudgetDimension::LogicalRequests)),
        "the scope's own token is what a budget stop stops"
    );
    assert_eq!(
        run.reason(),
        None,
        "the run keeps running after a scope's budget stop"
    );

    let held_run = CancellationToken::new();
    let held_scope = AnsweringScope::new(
        answering_limits(Some(4), None, None),
        CancellationToken::new(),
    );
    let held_sink = RecordingSink::new(held_run.clone());
    let (started, started_rx) = oneshot::channel();
    let provider = HeldProvider {
        started: Mutex::new(Some(started)),
    };
    let approval = AllowReadOnlyApproval;
    let executor = RecordingExecutor::default();
    let work = run_agent_with_control(
        &provider,
        &executor,
        request(),
        definitions(),
        AgentRunControl::new(
            AgentLimits::default(),
            answering_limits(Some(4), None, None),
            &approval,
            &held_sink,
            held_run.clone(),
        )
        .with_answering_scope(held_scope.clone()),
    );
    tokio::pin!(work);
    tokio::select! {
        result = &mut work => panic!("the held request returns only on a stop: {result:?}"),
        _ = started_rx => {}
    }

    held_run.cancel();

    let result = tokio::time::timeout(Duration::from_secs(1), &mut work)
        .await
        .expect("a stop of the run reaches the answering work");
    assert!(
        result.is_err(),
        "the stopped request cannot answer: {result:?}"
    );
    assert_eq!(
        held_scope.cancellation_token().reason(),
        Some(CancellationReason::User),
        "the scope's token inherits the run's stop"
    );
}

/// A provider that holds its request open until the cancellation it was given
/// stops — the shape a budget stop must interrupt.
struct HeldProvider {
    started: Mutex<Option<oneshot::Sender<()>>>,
}

#[async_trait]
impl ChatProvider for HeldProvider {
    fn name(&self) -> &str {
        "held-answering-turn"
    }

    async fn complete(&self, _: ChatRequest) -> Result<saya_agent::ChatResponse, ProviderError> {
        unreachable!("the loop receives through stream")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        cancellation: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        if let Some(started) = self.started.lock().unwrap().take() {
            let _ = started.send(());
        }
        cancellation.cancelled().await;
        Err(ProviderError::Cancelled)
    }
}
