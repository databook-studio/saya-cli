async fn run_prompt_after_question_start(
    request: SinkPrompt<'_>,
    answering_settled: Option<tokio::sync::oneshot::Sender<()>>,
) -> Result<AgentOutput, AgentRuntimeError> {
    let SinkPrompt {
        runtime,
        prompt,
        approval,
        can_prompt,
        can_obtain_approval,
        overrides,
        history,
        sink,
        cancellation,
        question_scope,
        state_db,
        decider,
        last_sql,
        session,
        agent_mode,
        capture,
        prior_tool_outcomes,
        continuation,
        compaction_summary,
    } = request;
    let inputs = match prepare_turn(
        runtime,
        &overrides,
        can_prompt,
        question_scope
            .as_ref()
            .and_then(|scope| scope.metrics())
            .as_deref(),
    )
    .await
    {
        Ok(inputs) => inputs,
        Err(error) => {
            if let Some(scope) = question_scope.as_ref() {
                scope.finish_segment();
            }
            return Err(with_next_step(error));
        }
    };
    run_prompt_with_continuation_inputs_with_signal(
        TurnPrompt {
            runtime,
            inputs,
            prompt,
            approval,
            can_prompt,
            can_obtain_approval,
            history,
            sink,
            cancellation,
            question_scope,
            state_db,
            decider,
            last_sql,
            session,
            agent_mode,
            capture,
            prior_tool_outcomes,
            continuation,
            compaction_summary,
        },
        answering_settled,
    )
    .await
}
