//! The serial tool path settles a failed call the same way the concurrent batch does: a failed
//! effect-free or proven no-effect call is a known call, anything else stays uncertain.

use saya_agent::{CancellationToken, EffectOutcome, ToolConcurrency, run_agent_with_control};

use super::{
    common::{ApproveAll, control, effect, request, tool, tool_call},
    executor::RecordingExecutor,
    owner::{OwnerOptions, RecordingOwner, tool as tool_settlement, trace},
    provider::{ScriptedProvider, answer, tool_calls},
};

/// Runs one serial call of `name` (declared with `effect`) through a failing executor and asserts
/// the tool settlement the owner recorded is `expected`.
async fn assert_serial_settlement(
    name: &str,
    effect: saya_agent::ToolEffect,
    configure: impl FnOnce(&mut RecordingExecutor),
    expected: EffectOutcome,
) {
    let trace = trace();
    let owner = RecordingOwner::new(trace.clone(), OwnerOptions::default());
    let provider = ScriptedProvider::new(
        vec![tool_calls(vec![tool_call(name, "call-1")]), answer("done")],
        trace.clone(),
    );
    let (mut executor, _) = RecordingExecutor::new(trace.clone());
    configure(&mut executor);
    let approval = ApproveAll::default();
    let control = control(&approval, CancellationToken::new()).with_effect_admission(&owner);
    let definition = tool(name, effect, ToolConcurrency::Serial);
    let output = run_agent_with_control(&provider, &executor, request(), vec![definition], control)
        .await
        .unwrap();
    let settlements = owner.settlements.lock().unwrap();
    assert_eq!(settlements.len(), 3, "model, tool, model");
    assert_eq!(output.answer, "done");
    assert_eq!(settlements[1].1, tool_settlement(expected));
}

#[tokio::test]
async fn a_failed_serial_read_only_tool_settles_as_succeeded() {
    assert_serial_settlement(
        "read",
        effect(true, false, false),
        |e| e.fail_name = Some("read".to_owned()),
        EffectOutcome::Succeeded,
    )
    .await;
}

#[tokio::test]
async fn a_serial_refusal_proven_to_change_nothing_settles_as_succeeded() {
    assert_serial_settlement(
        "write",
        effect(false, true, false),
        |e| e.fail_no_effect = Some("write".to_owned()),
        EffectOutcome::Succeeded,
    )
    .await;
}

#[tokio::test]
async fn a_failed_serial_mutating_tool_still_settles_as_unknown() {
    assert_serial_settlement(
        "write",
        effect(false, true, false),
        |e| e.fail_name = Some("write".to_owned()),
        EffectOutcome::Unknown,
    )
    .await;
}
