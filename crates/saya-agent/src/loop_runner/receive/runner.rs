use super::super::{
    add_usage, attempt, emit, preflight, receive_stream::stream_attempt, tool_protocol,
};
use super::{context::ReceiveContext, retry};
use crate::{
    AgentEvent, AgentMetric, AgentMetricCount, ChatMessage, ChatRequest, ProviderContinuationPhase,
    ProviderRecoveryPhase, ProviderRecoveryReason, TokenUsage, UsageCall,
};

pub(in crate::loop_runner) async fn receive(
    context: ReceiveContext<'_>,
) -> Result<(ChatMessage, TokenUsage, Option<String>), attempt::ReceiveFailure> {
    let ReceiveContext {
        provider,
        model,
        messages,
        ordinary_history_pairs,
        protected_prefix_len,
        definitions,
        context_byte_budget,
        request_preflight,
        continuation,
        sink,
        cancellation,
        events,
        admission,
    } = context;
    let mut recovered_usage = TokenUsage::default();
    if let Err(error) = admission.check(cancellation) {
        return Err(attempt::ReceiveFailure {
            error,
            usage: recovered_usage,
        });
    }
    if let Err(error) = preflight::fit_request(
        model,
        messages,
        ordinary_history_pairs,
        protected_prefix_len,
        definitions,
        context_byte_budget,
        request_preflight,
    ) {
        let error = match error {
            preflight::PreflightError::Budget(dimension) => {
                admission.stop_preflight(cancellation, dimension)
            }
            preflight::PreflightError::Serialization => {
                crate::AgentError::Limit("request projection serialization failed")
            }
        };
        return Err(attempt::ReceiveFailure {
            error,
            usage: recovered_usage,
        });
    }
    let request = ChatRequest {
        model: model.into(),
        messages: messages.as_slice().into(),
        tools: definitions.into(),
        ..Default::default()
    };
    let delays = crate::providers::default_retry_delays();
    let mut retries = 0_u8;
    if let Err(error) = admission.request(sink, cancellation).await {
        return Err(attempt::ReceiveFailure {
            error,
            usage: recovered_usage,
        });
    }
    if let Some(progress) = continuation {
        debug_assert!(progress.index > 0 && progress.index <= progress.limit);
        sink.metric(AgentMetric::Count(AgentMetricCount::AdmittedContinuation))
            .await;
        emit(
            events,
            sink,
            AgentEvent::provider_continuation(
                ProviderContinuationPhase::Continuing,
                progress.index as u32,
                progress.limit as u32,
            ),
        )
        .await;
        if let Err(error) = admission.check(cancellation) {
            return Err(attempt::ReceiveFailure {
                error,
                usage: recovered_usage,
            });
        }
    }
    loop {
        match stream_attempt(provider, &request, sink, cancellation, events, retries > 0).await {
            attempt::AttemptOutcome::Success {
                message,
                usage,
                reasoning,
            } => {
                let observation = admission
                    .observe_tools(message.tool_calls.len() as u64, sink, cancellation)
                    .await;
                if let Some(usage) = usage {
                    emit(events, sink, AgentEvent::usage(UsageCall::Answer, usage)).await;
                    add_usage(&mut recovered_usage, usage);
                }
                let settlement = admission.settle(usage, cancellation);
                if let Err(error) = observation {
                    return Err(attempt::ReceiveFailure {
                        error,
                        usage: recovered_usage,
                    });
                }
                if let Err(error) = settlement
                    && !matches!(&error, crate::AgentError::Limit("answering budget stopped"))
                {
                    return Err(attempt::ReceiveFailure {
                        error,
                        usage: recovered_usage,
                    });
                }
                if !tool_protocol::valid_ids(&message.tool_calls) {
                    emit(
                        events,
                        sink,
                        AgentEvent::provider_recovery(
                            ProviderRecoveryPhase::NotRetried,
                            ProviderRecoveryReason::ToolCallProtocol,
                            retries.saturating_add(1),
                            attempt::MAX_ATTEMPTS,
                        ),
                    )
                    .await;
                    return Err(attempt::ReceiveFailure {
                        error: crate::AgentError::InvalidToolCall,
                        usage: recovered_usage,
                    });
                }
                return Ok((message, recovered_usage, reasoning));
            }
            attempt::AttemptOutcome::Failed { error, usage } => {
                if let Some(failure) = retry::settle_failed_usage(
                    &error,
                    usage,
                    admission,
                    sink,
                    cancellation,
                    events,
                    &mut recovered_usage,
                )
                .await
                {
                    return Err(failure);
                }
                match retry::plan_retry(
                    error,
                    retries,
                    &delays,
                    admission,
                    sink,
                    cancellation,
                    events,
                )
                .await
                {
                    retry::RetryOutcome::Retry(next) => retries = next,
                    retry::RetryOutcome::GiveUp(error) => {
                        return Err(attempt::ReceiveFailure {
                            error,
                            usage: recovered_usage,
                        });
                    }
                }
            }
        }
    }
}
