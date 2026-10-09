//! The three bounded provider attempts the claim rides: a successful turn,
//! a terminal attempt whose collection limit trips with its receipt
//! reported, and one whose receipt is never reported.

use saya_agent::{AgentError, AgentLimits, CancellationToken, ProviderError, run_agent_with_sink};
use saya_harness::engine::EngineEventSink;

use super::fixtures::{NoTools, ScriptedProvider, request};

pub(super) async fn drive_bounded_attempts(
    provider: &ScriptedProvider,
    tools: &NoTools,
    sink: &EngineEventSink,
    cancellation: CancellationToken,
) {
    let output = run_agent_with_sink(
        provider,
        tools,
        request(),
        Vec::new(),
        AgentLimits::default(),
        &saya_agent::AllowReadOnlyApproval,
        sink,
        cancellation.clone(),
    )
    .await
    .unwrap();
    assert_eq!(output.answer, "done");
    let terminal = run_agent_with_sink(
        provider,
        tools,
        request(),
        Vec::new(),
        AgentLimits::default(),
        &saya_agent::AllowReadOnlyApproval,
        sink,
        cancellation,
    )
    .await;
    assert!(matches!(
        terminal,
        Err(AgentError::Provider(ProviderError::ToolCollectionLimit))
    ));
    let unreported = run_agent_with_sink(
        provider,
        tools,
        request(),
        Vec::new(),
        AgentLimits::default(),
        &saya_agent::AllowReadOnlyApproval,
        sink,
        CancellationToken::default(),
    )
    .await;
    assert!(matches!(
        unreported,
        Err(AgentError::Provider(ProviderError::ToolCollectionLimit))
    ));
}
