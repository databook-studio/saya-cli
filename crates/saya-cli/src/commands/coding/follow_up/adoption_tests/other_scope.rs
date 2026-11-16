use super::{request, resume};
use crate::{
    ChildFollowUpRequest, CodingApplication, CodingTask,
    coding_test_support::follow_up_fixture::Fixture,
};
use saya_types::AgentId;
use std::{num::NonZeroU64, sync::Arc, time::Duration};
pub(super) async fn check(f: &Fixture, app: &Arc<CodingApplication>, causal: NonZeroU64) {
    let job = resume(app, f.job.job_id()).await;
    let before = job.ledger.snapshot().await;
    let agent = AgentId::parse("independent-b").unwrap();
    job.delegate(vec![CodingTask {
        agent_id: agent.clone(),
        subagent_type: "general".into(),
        task: "known independent owned child".into(),
        write_paths: vec!["extra.txt".into()],
        dependencies: vec![],
    }])
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(15), job.join())
        .await
        .unwrap()
        .unwrap();
    let status = job.status().await.unwrap();
    let child = status["children"]
        .as_array()
        .unwrap()
        .iter()
        .find(|child| child["agent_id"] == agent.as_str())
        .unwrap();
    assert_eq!(child["write_paths"], serde_json::json!(["extra.txt"]));
    let completion = f
        .store
        .list_coding_completion_history(job.job_id(), None, 32)
        .await
        .unwrap()
        .items
        .into_iter()
        .find(|item| item.result.agent_id() == &agent)
        .unwrap();
    let result = job
        .follow_up(ChildFollowUpRequest {
            agent_id: agent.clone(),
            expected_completion_sequence: completion.sequence,
            instruction: "bounded independent B repair".into(),
        })
        .await;
    if result.is_ok() {
        tokio::time::timeout(Duration::from_secs(15), job.join())
            .await
            .unwrap()
            .unwrap();
    }
    assert!(
        result.is_ok(),
        "reconciled A root usage must not permanently block known owned B: {result:?}"
    );
    assert_eq!(
        job.ledger.snapshot().await.unknown_tool_calls,
        before.unknown_tool_calls
    );
    assert_eq!(
        job.ledger.snapshot().await.unsettled_reservations,
        before.unsettled_reservations
    );
    assert!(
        f.store
            .get_coding_budget_reservation(job.job_id(), causal)
            .await
            .unwrap()
            .unwrap()
            .reconciled
    );
    assert_eq!(
        f.store
            .coding_child_invocation_history(job.job_id(), &agent)
            .await
            .unwrap()
            .len(),
        2
    );
    assert!(
        job.follow_up(request(f, "wrong stale A brief"))
            .await
            .is_err()
    );
    job.finalize_run(Ok(f.output.clone())).await.unwrap();
}
