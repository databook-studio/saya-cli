use super::super::{ChildFollowUpRequest, CodingApplication, CodingAuthorization, NativeCodingJob};
use crate::coding_test_support::{
    follow_up_fixture::Fixture, follow_up_provider::Provider, isolation,
};
use saya_agent::{
    CancellationToken, EffectAdmission, EffectOutcome, EffectSettlement, LocalStateEffect,
    ToolEffect,
};
use saya_types::{
    BudgetCharge, BudgetDimension, BudgetReservationV1, CodingEventKind, CodingEventV1,
    Sha256Digest,
};
#[path = "adoption_tests/model_usage.rs"]
mod model_usage;
#[path = "adoption_tests/other_scope.rs"]
mod other_scope;
#[path = "adoption_tests/positive.rs"]
mod positive;
use std::{num::NonZeroU64, sync::Arc};
#[test]
fn recovered_root_usage_does_not_block_independent_owned_child() {
    run(
        "adopt-b",
        "recovered_root_usage_does_not_block_independent_owned_child",
    );
}
#[test]
fn reconciled_effects_and_historical_unknown_usage_are_distinct_in_context() {
    run(
        "adopt-context",
        "reconciled_effects_and_historical_unknown_usage_are_distinct_in_context",
    );
}
#[test]
fn exact_prepared_root_crash_adopts_and_allows_later_bounded_continuation() {
    run(
        "adopt",
        "exact_prepared_root_crash_adopts_and_allows_later_bounded_continuation",
    );
}
#[test]
fn a_given_up_model_request_does_not_stop_the_exact_prepared_root_adoption() {
    run(
        "adopt-model",
        "a_given_up_model_request_does_not_stop_the_exact_prepared_root_adoption",
    );
}
#[test]
fn prepared_invocation_with_new_child_reservation_refuses_adoption() {
    run(
        "adopt-child",
        "prepared_invocation_with_new_child_reservation_refuses_adoption",
    );
}
#[test]
fn interposed_known_admission_breaks_causal_recovery_proof() {
    run(
        "adopt-interposed",
        "interposed_known_admission_breaks_causal_recovery_proof",
    );
}
#[test]
fn noncausal_reconciled_root_tool_is_not_forgiven() {
    run(
        "adopt-other",
        "noncausal_reconciled_root_tool_is_not_forgiven",
    );
}
fn run(case: &str, name: &str) {
    let test = format!("commands::coding::follow_up::adoption_tests::{name}");
    if isolation::run_parent(case, &test) {
        return;
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(tokio::task::LocalSet::new().run_until(exercise(case)));
}
fn request(f: &Fixture, instruction: &str) -> ChildFollowUpRequest {
    ChildFollowUpRequest {
        agent_id: f.completion.result.agent_id().clone(),
        expected_completion_sequence: f.completion.sequence,
        instruction: instruction.into(),
    }
}
async fn resume(app: &Arc<CodingApplication>, job: &saya_types::JobId) -> Arc<NativeCodingJob> {
    let approval = CodingAuthorization::scoped(&app.propose_resume(job).await.unwrap()).unwrap();
    app.resume(job, &approval, CancellationToken::new())
        .await
        .unwrap()
}
async fn reconcile(app: &CodingApplication, job: &saya_types::JobId) {
    let plan = app.reconcile_plan(job).await.unwrap();
    let digest = Sha256Digest::parse(plan["plan_sha256"].as_str().unwrap()).unwrap();
    let result = app.reconcile(job, &digest).await.unwrap();
    assert_eq!(result["refunded"], false);
}
async fn exercise(case: &str) {
    let root = crate::state_path::state_db_path()
        .parent()
        .unwrap()
        .to_path_buf();
    let provider = Provider::new();
    let f = if case == "adopt-b" {
        Fixture::with_paths(
            &root,
            &provider,
            vec!["tracked.txt".into(), "extra.txt".into()],
        )
        .await
    } else {
        Fixture::new(&root, &provider).await
    };
    assert!(!f.output.truncated);
    let app = CodingApplication::session(
        f.runtime.clone(),
        f.store.clone(),
        &f.repository,
        crate::cli::RootMode::Managed,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let initial = resume(&app, f.job.job_id()).await;
    let view = initial.ledger.root_view();
    if case == "adopt-model" {
        model_usage::give_up_request(&view).await;
    }
    let effect = ToolEffect {
        database_data: false,
        external_side_effect: false,
        requires_approval: true,
        local_state: LocalStateEffect::WriteWorkspace,
    };
    let permits = view.admit_tool_batch(&[effect]).await.unwrap();
    let reservations = f
        .store
        .list_coding_budget_reservations(initial.job_id(), None, 100)
        .await
        .unwrap();
    let causal = reservations
        .items
        .last()
        .unwrap()
        .reservation
        .reservation_sequence();
    if case == "adopt-interposed" {
        let interposed = view.admit_tool_batch(&[effect]).await.unwrap();
        view.settle(
            interposed[0],
            EffectSettlement::Tool {
                outcome: EffectOutcome::NotStarted,
            },
        )
        .await
        .unwrap();
    }
    let lease = initial.operation.lock().await;
    let prepared = initial
        .prepare_follow_up(request(&f, "bounded exact repair"))
        .await
        .unwrap();
    let invocation = prepared.receipt.invocation_id.clone();
    let history = f
        .store
        .coding_child_invocation_history(initial.job_id(), f.completion.result.agent_id())
        .await
        .unwrap();
    if case == "adopt-interposed" {
        assert_ne!(history.last().unwrap().sequence().get(), causal.get() + 1);
    } else {
        assert_eq!(
            history.last().unwrap().sequence().get(),
            causal.get() + 1,
            "actual root permit directly precedes native preparation"
        );
    }
    drop(prepared);
    drop(lease);
    if case == "adopt-other" {
        let extra = view.admit_tool_batch(&[effect]).await.unwrap();
        drop(extra);
    }
    drop(permits);
    drop(view);
    drop(initial);
    reconcile(&app, f.job.job_id()).await;
    let restored = resume(&app, f.job.job_id()).await;
    let before = restored.ledger.snapshot().await;
    assert!(before.unknown_tool_calls >= 1 && before.unsettled_reservations >= 1);
    if case == "adopt-child" {
        let last = f
            .store
            .coding_recovery_status(restored.job_id())
            .await
            .unwrap()
            .last_sequence;
        let sequence = NonZeroU64::new(last + 1).unwrap();
        let reservation = BudgetReservationV1::new(
            restored.job_id().clone(),
            sequence,
            Some(f.completion.result.agent_id().clone()),
            Some(invocation.clone()),
            vec![
                BudgetCharge::new(
                    BudgetDimension::ToolCalls,
                    None,
                    None,
                    NonZeroU64::new(1).unwrap(),
                )
                .unwrap(),
            ],
        )
        .unwrap();
        let event = CodingEventV1::new(
            sequence,
            restored.job_id().clone(),
            Some(f.completion.result.agent_id().clone()),
            Some(invocation.clone()),
            CodingEventKind::BudgetReserved {
                reservation: Box::new(reservation),
            },
        )
        .unwrap();
        let authority = saya_store::CodingRuntimeAuthority {
            epoch: restored.epoch,
            policy_digest: restored.policy_digest.clone(),
        };
        f.store
            .reserve_coding_budget_fenced(event, Some(&authority))
            .await
            .unwrap();
        drop(restored);
        reconcile(&app, f.job.job_id()).await;
        let restored = resume(&app, f.job.job_id()).await;
        assert_eq!(
            restored.ledger.snapshot().await.unknown_tool_calls,
            before.unknown_tool_calls + 1
        );
        assert!(
            restored
                .follow_up(request(&f, "bounded exact repair"))
                .await
                .is_err()
        );
        assert!(
            f.store
                .get_coding_budget_reservation(f.job.job_id(), sequence)
                .await
                .unwrap()
                .unwrap()
                .reconciled
        );
        return;
    }
    if case == "adopt-interposed" {
        assert_eq!(before.unknown_tool_calls, 1);
        assert!(
            restored
                .follow_up(request(&f, "bounded exact repair"))
                .await
                .is_err()
        );
        return;
    }
    if case == "adopt-other" {
        assert_eq!(before.unknown_tool_calls, 2);
        assert!(
            restored
                .follow_up(request(&f, "bounded exact repair"))
                .await
                .is_err()
        );
        return;
    }
    if case == "adopt-context" {
        let blocks = restored.continuation_context().await.unwrap();
        let context: serde_json::Value = serde_json::from_str(&blocks[0].body).unwrap();
        assert_eq!(
            context["budget"]["unresolved_effects"], 0,
            "reconciled effects must not be labeled unresolved"
        );
        assert_eq!(context["budget"]["historical_unsettled_reservations"], 1);
        assert_eq!(context["budget"]["historical_unknown_tool_calls"], 1);
        return;
    }
    positive::check(&f, &provider, &restored, before, causal, invocation, effect).await;
    if case == "adopt-b" {
        other_scope::check(&f, &app, causal).await;
    }
}
