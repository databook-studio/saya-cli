pub(crate) async fn run_prompt_with_sink_and_outcomes(
    mut request: SinkPrompt<'_>,
) -> Result<AgentOutput, AgentRuntimeError> {
    if let Some(scope) = request.question_scope.as_ref() {
        request.cancellation = scope.cancellation();
    }
    let cancellation = request.cancellation.clone();
    if cancellation.is_cancelled() {
        return Err(AgentRuntimeError::Agent("request cancelled".into()));
    }
    let scope = request.question_scope.clone();
    let deadline = match scope.as_ref() {
        Some(scope) => scope.begin_segment(),
        None => finite_seconds(
            request
                .runtime
                .resolved
                .investigation_budgets
                .elapsed_seconds,
        ),
    };
    if cancellation.is_cancelled() {
        if let Some(scope) = scope.as_ref() {
            scope.finish_segment();
        }
        return Err(AgentRuntimeError::Agent("request cancelled".into()));
    }
    let (answering_settled_tx, mut answering_settled) = tokio::sync::oneshot::channel();
    let work = run_prompt_after_question_start(request, Some(answering_settled_tx));
    tokio::pin!(work);
    let result = match deadline {
        Some(duration) => tokio::select! {
            biased;
            result = &mut work => result,
            _ = &mut answering_settled => work.await,
            _ = tokio::time::sleep(duration) => {
                cancellation.cancel_with_reason(CancellationReason::Budget(BudgetDimension::ElapsedTime));
                work.await
            }
        },
        None => work.await,
    };
    // The answering owner normally finishes before learning. This is the
    // idempotent fallback for an earlier preparation or composition error.
    if let Some(scope) = scope {
        scope.finish_segment();
    }
    result
}
