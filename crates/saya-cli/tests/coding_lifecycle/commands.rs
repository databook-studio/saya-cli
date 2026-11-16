//! The command surface a helper owns is `shell`: a helper runs a real command
//! through it and the run keeps the real exit it produced. There is no
//! typed-argv runner (`run_program` or a `coding_command_start`/
//! `coding_command_wait` pair). Each admitted command charges one command slot,
//! and the charge settles exactly once.
use super::{provider::Provider, support::Fixture};
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingJobRequest};
use saya_types::JobId;
// Only the sandbox-lane charge test reads these; the Linux counterpart does not.
#[cfg(target_os = "macos")]
use saya_types::{BudgetDimension, BudgetEffectState, BudgetObservationValue};
use serde_json::Value;
use std::collections::BTreeMap;

/// The actor a captured request belongs to, read the way the transport does:
/// the helper's brief names it after `write for `.
fn actor(body: &str) -> String {
    body.split("UNTRUSTED CHILD TASK")
        .nth(1)
        .and_then(|task| task.split("write for ").nth(1))
        .and_then(|tail| {
            let suffix = tail.strip_prefix("child-")?;
            let end = suffix
                .find(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-')
                .unwrap_or(suffix.len());
            (end > 0).then_some(&tail[.."child-".len() + end])
        })
        .unwrap_or("parent")
        .to_owned()
}

/// Every helper's own `shell` result, keyed by the helper that ran it: each
/// command's exit code and settlement facts, as the child saw them.
fn shell_results(provider: &Provider) -> BTreeMap<String, Value> {
    let mut results = BTreeMap::new();
    for body in provider.bodies.lock().unwrap().iter() {
        let actor = actor(body);
        let request: Value = serde_json::from_str(body).unwrap();
        for message in request["messages"].as_array().cloned().unwrap_or_default() {
            let Some(content) = message["content"].as_str() else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<Value>(content) else {
                continue;
            };
            if value.get("exit_code").is_some() {
                results.insert(actor.clone(), value);
            }
        }
    }
    results
}

// A helper's `shell` runs in the sandboxed lane, which only macOS proves. On
// Linux the same call falls back to the host lane, where a headless job cannot
// obtain approval; the Linux counterpart below pins that refusal instead.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn a_helpers_failing_shell_command_keeps_its_real_nonzero_exit() {
    // Mode 1: the root hands one piece to each of two helpers; each helper runs
    // the real failing command through its own `shell`.
    let provider = Provider::checks();
    let fixture = Fixture::with_endpoint("native-check", Some(&provider.endpoint)).await;
    let request = CodingJobRequest::new(
        JobId::parse("native-check").unwrap(),
        "Run a failing native command with helpers".into(),
        vec!["new-a.txt".into(), "new-b.txt".into()],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(60), job.run(&NoopEventSink))
        .await
        .unwrap()
        .unwrap();

    let results = shell_results(&provider);
    assert_eq!(
        results.keys().cloned().collect::<Vec<_>>(),
        vec!["child-a".to_owned(), "child-b".to_owned()],
        "both helpers ran their command: {results:?}"
    );
    for (actor, result) in &results {
        assert_eq!(
            result["exit_code"], 1,
            "{actor} kept the real nonzero exit: {result}"
        );
        // A false result flag is left out of the shape, so an absent
        // `cancelled` or `timed_out` reads as that default: the run neither
        // cancelled nor timed out. A cancelled run is an error, and a
        // timed-out one carries the flag, so only the default is absent.
        for flag in ["cancelled", "timed_out"] {
            assert!(
                !result[flag].as_bool().unwrap_or(false),
                "{actor} carries a true {flag}: {result}"
            );
        }
    }
    let completions = fixture
        .store
        .list_coding_completion_history(job.job_id(), None, 32)
        .await
        .unwrap();
    assert_eq!(
        completions.items.len(),
        2,
        "each helper's own completion is durable"
    );
}

// Sandboxed `shell` again: each command must actually run to settle a charge,
// and only macOS can prove the sandbox lane.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn every_shell_command_settles_its_charge_exactly_once() {
    // Mode 24: one helper runs a sequence of real `shell` commands. Each
    // admitted tool call charges one tool call, and the charge settles exactly
    // once with the call's own effect observed. The retired command pair's
    // receipt retention is gone with the typed-argv runner; what survives is
    // that each admitted command settles once.
    let provider = Provider::command_wait_sequence();
    let fixture = Fixture::with_endpoint("native-command-wait", Some(&provider.endpoint)).await;
    let request = CodingJobRequest::new(
        JobId::parse("native-command-wait").unwrap(),
        "Exercise the helper's shell command surface".into(),
        vec!["new-a.txt".into(), "new-b.txt".into()],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(120), job.run(&NoopEventSink))
        .await
        .unwrap()
        .unwrap();

    let reservations = fixture
        .store
        .list_coding_budget_reservations(job.job_id(), None, 100)
        .await
        .unwrap();
    let tool_calls = reservations
        .items
        .iter()
        .filter(|entry| {
            entry
                .reservation
                .charges()
                .iter()
                .any(|charge| charge.dimension() == BudgetDimension::ToolCalls)
        })
        .collect::<Vec<_>>();
    assert!(
        tool_calls.len() >= 2,
        "the helper's shell sequence charged tool calls"
    );
    for entry in tool_calls {
        let settlement = entry.settlement.as_ref().expect("tool call settled");
        assert_eq!(settlement.effect_state(), BudgetEffectState::Settled);
        assert_eq!(settlement.observations().len(), 1);
        assert_eq!(
            settlement.observations()[0].dimension(),
            BudgetDimension::ToolCalls
        );
        assert_eq!(
            settlement.observations()[0].value(),
            BudgetObservationValue::Measured(1),
            "each admitted command ran before its settlement"
        );
    }
    let completions = fixture
        .store
        .list_coding_completion_history(job.job_id(), None, 32)
        .await
        .unwrap();
    assert_eq!(
        completions.items.len(),
        1,
        "the helper's own completion is durable"
    );
    assert_eq!(
        fixture
            .store
            .coding_recovery_status(job.job_id())
            .await
            .unwrap()
            .unresolved_reservations,
        0
    );
}

// The Linux counterpart: the sandboxed lane is not offered here, so a helper's
// `shell` is refused with the documented host-approval message instead of
// running. The helpers still finish, and each completion is durable.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_helpers_shell_refuses_on_the_host_lane_with_the_documented_message() {
    let provider = Provider::checks();
    let fixture = Fixture::with_endpoint("native-check-host", Some(&provider.endpoint)).await;
    let request = CodingJobRequest::new(
        JobId::parse("native-check-host").unwrap(),
        "Run a failing native command with helpers".into(),
        vec!["new-a.txt".into(), "new-b.txt".into()],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(60), job.run(&NoopEventSink))
        .await
        .unwrap()
        .unwrap();

    assert!(
        shell_results(&provider).is_empty(),
        "the sandboxed lane is not offered on Linux, so no command produced an exit"
    );
    {
        let bodies = provider.bodies.lock().unwrap();
        assert!(
            bodies
                .iter()
                .any(|body| body.contains("no operator is available")),
            "the refusal carries the documented host-approval message"
        );
    }
    let completions = fixture
        .store
        .list_coding_completion_history(job.job_id(), None, 32)
        .await
        .unwrap();
    assert_eq!(
        completions.items.len(),
        2,
        "each helper's own completion is durable"
    );
}
