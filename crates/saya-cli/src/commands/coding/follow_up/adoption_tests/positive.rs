use super::request;
use crate::{
    ChildFollowUpRequest, NativeCodingJob,
    coding_test_support::{follow_up_fixture::Fixture, follow_up_provider::Provider},
};
use saya_agent::{EffectAdmission, EffectOutcome, EffectSettlement, ToolEffect};
use saya_harness::coding::budget::BudgetSnapshot;
use saya_types::InvocationId;
use std::{
    num::NonZeroU64,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
pub(super) async fn check(
    f: &Fixture,
    provider: &Provider,
    restored: &Arc<NativeCodingJob>,
    before: BudgetSnapshot,
    causal: NonZeroU64,
    invocation: InvocationId,
    effect: ToolEffect,
) {
    assert!(
        restored
            .follow_up(request(f, "wrong repair brief"))
            .await
            .is_err()
    );
    let checkouts = f
        .store
        .list_managed_checkouts(restored.job_id(), None, 100)
        .await
        .unwrap();
    // Helpers share the root's working checkout W: a child owns no
    // checkout of its own, so the source the follow-up binds is W's — the job's
    // agent-less working checkout.
    let working = checkouts
        .items
        .iter()
        .find(|item| item.ownership.agent_id().is_none())
        .unwrap();
    let path = std::path::Path::new(working.ownership.managed_path().as_str()).join("tracked.txt");
    let original = std::fs::read(&path).unwrap();
    std::fs::write(&path, b"changed fixture source\n").unwrap();
    assert!(
        restored
            .follow_up(request(f, "bounded exact repair"))
            .await
            .is_err()
    );
    std::fs::write(&path, original).unwrap();
    let current_view = restored.ledger.root_view();
    let current_permits = current_view.admit_tool_batch(&[effect]).await.unwrap();
    let result = restored.follow_up(request(f, "bounded exact repair")).await;
    current_view
        .settle(
            current_permits[0],
            EffectSettlement::Tool {
                outcome: if result.is_ok() {
                    EffectOutcome::Succeeded
                } else {
                    EffectOutcome::NotStarted
                },
            },
        )
        .await
        .unwrap();
    provider.release.store(true, Ordering::Release);
    if result.is_ok() {
        let mut read_effect = effect;
        read_effect.requires_approval = false;
        read_effect.local_state = saya_agent::LocalStateEffect::None;
        let awaiting = current_view.admit_tool_batch(&[read_effect]).await.unwrap();
        assert_eq!(restored.ledger.snapshot().await.tool_permits_pending, 1);
        let joined = tokio::time::timeout(Duration::from_secs(15), restored.join())
            .await
            .unwrap()
            .unwrap();
        current_view
            .settle(
                awaiting[0],
                EffectSettlement::Tool {
                    outcome: EffectOutcome::Succeeded,
                },
            )
            .await
            .unwrap();
        assert!(
            !joined.effects_uncertain,
            "known child join is not uncertain solely from reconciled root usage"
        );
    }
    assert!(
        result.is_ok(),
        "exact actual prepared-root recovery must adopt: {result:?}"
    );
    assert_eq!(result.unwrap().invocation_id, invocation);
    let after = restored.ledger.snapshot().await;
    assert_eq!(after.unknown_tool_calls, before.unknown_tool_calls);
    assert_eq!(after.unsettled_reservations, before.unsettled_reservations);
    assert!(after.tool_calls >= before.tool_calls);
    let completion = f
        .store
        .list_coding_completion_history(restored.job_id(), None, 32)
        .await
        .unwrap()
        .items
        .pop()
        .unwrap();
    let next = restored
        .follow_up(ChildFollowUpRequest {
            agent_id: completion.result.agent_id().clone(),
            expected_completion_sequence: completion.sequence,
            instruction: "next bounded repair".into(),
        })
        .await;
    if next.is_ok() {
        tokio::time::timeout(Duration::from_secs(15), restored.join())
            .await
            .unwrap()
            .unwrap();
    }
    assert!(
        next.is_ok(),
        "reconciled historical root metric must not permanently disable legitimate continuation: {next:?}"
    );
    assert_eq!(
        f.store
            .coding_child_invocation_history(restored.job_id(), f.completion.result.agent_id())
            .await
            .unwrap()
            .len(),
        3
    );
    assert!(
        f.store
            .get_coding_budget_reservation(restored.job_id(), causal)
            .await
            .unwrap()
            .unwrap()
            .reconciled
    );
    restored.finalize_run(Ok(f.output.clone())).await.unwrap();
}
