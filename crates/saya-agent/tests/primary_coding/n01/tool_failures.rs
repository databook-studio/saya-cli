use std::sync::atomic::Ordering;

use saya_agent::{CancellationToken, EffectOutcome, ToolConcurrency, run_agent_with_control};

use super::{
    common::{ApproveAll, control, effect, request, tool, tool_call},
    executor::RecordingExecutor,
    owner::{OwnerOptions, RecordingOwner, model, tool as tool_settlement, trace},
    provider::{ScriptedProvider, answer, tool_calls},
};

#[tokio::test]
async fn a_failed_read_only_tool_settles_as_succeeded() {
    let trace = trace();
    let owner = RecordingOwner::new(trace.clone(), OwnerOptions::default());
    let provider = ScriptedProvider::new(
        vec![
            tool_calls(vec![
                tool_call("read", "call-1"),
                tool_call("read", "call-2"),
            ]),
            answer("done"),
        ],
        trace.clone(),
    );
    let (mut executor, executor_log) = RecordingExecutor::new(trace.clone());
    executor.fail_name = Some("read".to_owned());
    let approval = ApproveAll::default();
    let control = control(&approval, CancellationToken::new()).with_effect_admission(&owner);
    let definition = tool(
        "read",
        effect(true, false, false),
        ToolConcurrency::Concurrent,
    );

    let output = run_agent_with_control(&provider, &executor, request(), vec![definition], control)
        .await
        .unwrap();

    assert_eq!(output.answer, "done");
    assert_eq!(executor_log.calls.load(Ordering::SeqCst), 2);
    let settlements = owner.settlements.lock().unwrap();
    assert_eq!(settlements.len(), 4);
    assert_eq!(settlements[1].0, 2);
    assert_eq!(settlements[1].1, tool_settlement(EffectOutcome::Succeeded));
    assert_eq!(settlements[2].0, 3);
    assert_eq!(settlements[2].1, tool_settlement(EffectOutcome::Succeeded));
    assert_eq!(settlements[3].0, 4);
    assert_eq!(
        settlements[3].1,
        model(EffectOutcome::Succeeded, None, false)
    );
}

#[tokio::test]
async fn a_failed_mutating_tool_still_settles_as_unknown() {
    let trace = trace();
    let owner = RecordingOwner::new(trace.clone(), OwnerOptions::default());
    let provider = ScriptedProvider::new(
        vec![
            tool_calls(vec![
                tool_call("write", "call-1"),
                tool_call("write", "call-2"),
            ]),
            answer("done"),
        ],
        trace.clone(),
    );
    let (mut executor, executor_log) = RecordingExecutor::new(trace.clone());
    executor.fail_name = Some("write".to_owned());
    let approval = ApproveAll::default();
    let control = control(&approval, CancellationToken::new()).with_effect_admission(&owner);
    let definition = tool(
        "write",
        effect(false, true, false),
        ToolConcurrency::Concurrent,
    );

    let output = run_agent_with_control(&provider, &executor, request(), vec![definition], control)
        .await
        .unwrap();

    assert_eq!(output.answer, "done");
    assert_eq!(executor_log.calls.load(Ordering::SeqCst), 2);
    let settlements = owner.settlements.lock().unwrap();
    assert_eq!(settlements.len(), 4);
    assert_eq!(settlements[1].0, 2);
    assert_eq!(settlements[1].1, tool_settlement(EffectOutcome::Unknown));
    assert_eq!(settlements[2].0, 3);
    assert_eq!(settlements[2].1, tool_settlement(EffectOutcome::Unknown));
    assert_eq!(settlements[3].0, 4);
    assert_eq!(
        settlements[3].1,
        model(EffectOutcome::Succeeded, None, false)
    );
}

#[tokio::test]
async fn a_failed_read_only_tool_with_an_external_effect_stays_unknown() {
    let trace = trace();
    let owner = RecordingOwner::new(trace.clone(), OwnerOptions::default());
    let provider = ScriptedProvider::new(
        vec![
            tool_calls(vec![
                tool_call("read-external", "call-1"),
                tool_call("read-external", "call-2"),
            ]),
            answer("done"),
        ],
        trace.clone(),
    );
    let (mut executor, executor_log) = RecordingExecutor::new(trace.clone());
    executor.fail_name = Some("read-external".to_owned());
    let approval = ApproveAll::default();
    let control = control(&approval, CancellationToken::new()).with_effect_admission(&owner);
    let mut definition = tool(
        "read-external",
        effect(false, true, false),
        ToolConcurrency::Concurrent,
    );
    definition.read_only = true;

    let output = run_agent_with_control(&provider, &executor, request(), vec![definition], control)
        .await
        .unwrap();

    assert_eq!(output.answer, "done");
    assert_eq!(executor_log.calls.load(Ordering::SeqCst), 2);
    let settlements = owner.settlements.lock().unwrap();
    assert_eq!(settlements.len(), 4);
    assert_eq!(settlements[1].0, 2);
    assert_eq!(settlements[1].1, tool_settlement(EffectOutcome::Unknown));
    assert_eq!(settlements[2].0, 3);
    assert_eq!(settlements[2].1, tool_settlement(EffectOutcome::Unknown));
    assert_eq!(settlements[3].0, 4);
    assert_eq!(
        settlements[3].1,
        model(EffectOutcome::Succeeded, None, false)
    );
}
