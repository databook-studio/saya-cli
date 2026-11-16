//! The child transport must retain known refusal facts after budget stopping.
use super::{drain, support};
use saya_agent::{BudgetDimension, CancellationReason, CancellationToken, ToolDiagnosticStatus};
use saya_harness::coding::{
    child::ChildExecutionSpec,
    coordinator::{ChildEventKind, child_event_channel, run_child},
};
use std::sync::atomic::Ordering;

#[tokio::test]
async fn budget_refusal_keeps_child_diagnostics_and_admitted_identity() {
    let factory = support::ScenarioFactory::new(1);
    let (limits, mut answering) = support::limits(2);
    answering.max_tool_calls = Some(0);
    let spec = ChildExecutionSpec::new(
        support::authority_view("agent-budget", "invocation-budget"),
        support::request(),
        limits,
        answering,
        None,
    )
    .unwrap();
    let (sender, receiver) = child_event_channel(&spec);
    let cancellation = CancellationToken::new();
    let (output, events) = tokio::join!(
        run_child(spec, &factory, sender, cancellation.clone()),
        drain(receiver),
    );
    assert!(output.unwrap().truncated);
    assert_eq!(factory.stats.executor_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        cancellation.reason(),
        Some(CancellationReason::Budget(
            BudgetDimension::RequestedToolCalls
        ))
    );
    let diagnostics = events
        .iter()
        .filter_map(|event| match event.kind() {
            ChildEventKind::ToolDiagnostic { diagnostic } => Some((event, diagnostic)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        diagnostics
            .iter()
            .map(|(_, diagnostic)| diagnostic.status())
            .collect::<Vec<_>>(),
        vec![
            ToolDiagnosticStatus::Requested,
            ToolDiagnosticStatus::Refused
        ]
    );
    for (event, diagnostic) in diagnostics {
        assert_eq!(event.job_id().as_str(), "job-a");
        assert_eq!(event.agent_id().as_str(), "agent-budget");
        assert_eq!(event.invocation_id().as_str(), "invocation-budget");
        assert_eq!(diagnostic.tool(), "inspect");
    }
}
