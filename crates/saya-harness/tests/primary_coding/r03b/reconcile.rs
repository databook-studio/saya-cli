//! Reconciliation of a crashed job's unresolved budget effects behind an
//! exact plan digest: what blocks resume until reconciled, what refuses
//! to reconcile at all, and that a reconciled charge is never refunded.

use std::num::NonZeroU64;
use std::time::Duration;

use saya_agent::EffectAdmission;
use saya_harness::coding::coordinator::{
    ReconcileError, ResumeError, plan_reconciliation, reconcile_effects, resume_job,
};
use saya_store::CodingRuntimeAuthority;
use saya_types::{CodingEventKind, JobState, Sha256Digest};

use super::support::{Fixture, child_event, job_event, limits};

fn fresh_authority(epoch: u64) -> CodingRuntimeAuthority {
    CodingRuntimeAuthority {
        epoch,
        policy_digest: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
    }
}

#[tokio::test]
async fn a_crashed_effect_blocks_resume_until_reconciled_and_is_never_refunded() {
    let fixture = Fixture::new("reconcile-crash").await;
    let owner_limits = limits(8, 8, 8, 2, 1, Duration::from_secs(30));
    let ledger = fixture.ledger(owner_limits).await.unwrap();
    let _permit = ledger.root_view().admit_model_request().await.unwrap();
    fixture.close_store().await;
    drop(ledger);

    let store = fixture.reopen();
    let crashed = store.coding_recovery_status(&fixture.job).await.unwrap();
    assert_eq!(crashed.unresolved_reservations, 1);
    let refused = resume_job(
        store.clone(),
        fixture.job.clone(),
        owner_limits,
        fresh_authority(2),
    )
    .await;
    assert!(
        matches!(refused, Err(ResumeError::ReconciliationRequired(status)) if status.unresolved_reservations == 1)
    );

    let plan = plan_reconciliation(&store, &fixture.job).await.unwrap();
    assert_eq!(plan.job, fixture.job);
    assert_eq!(plan.last_sequence, 2);
    assert_eq!(plan.effects.len(), 1);
    assert_eq!(plan.effects[0].reservation_sequence.get(), 2);
    assert!(!plan.effects[0].settled_uncertain);
    assert_eq!(plan.effects[0].reserved.len(), 2);

    let report = reconcile_effects(&store, &fixture.job, &plan.digest)
        .await
        .unwrap();
    assert_eq!(report.reconciled, vec![NonZeroU64::new(2).unwrap()]);

    let status = store.coding_recovery_status(&fixture.job).await.unwrap();
    assert_eq!(status.unresolved_reservations, 0);
    let resumed = resume_job(
        store.clone(),
        fixture.job.clone(),
        owner_limits,
        fresh_authority(2),
    )
    .await
    .unwrap();
    let snapshot = resumed.snapshot().await;
    assert_eq!(snapshot.logical_requests, 1);
    assert_eq!(snapshot.model_slots_active, 1);
    fixture.close().await;
}

#[tokio::test]
async fn a_stale_plan_digest_is_refused_and_writes_nothing() {
    let fixture = Fixture::new("reconcile-stale").await;
    let owner_limits = limits(8, 8, 8, 4, 1, Duration::from_secs(30));
    let ledger = fixture.ledger(owner_limits).await.unwrap();
    let _first = ledger.root_view().admit_model_request().await.unwrap();
    let stale = plan_reconciliation(&fixture.store, &fixture.job)
        .await
        .unwrap();

    // The stream changes after the plan was taken: a second request is
    // admitted and its reservation is appended.
    let _second = ledger.root_view().admit_model_request().await.unwrap();
    let before = fixture
        .store
        .coding_recovery_status(&fixture.job)
        .await
        .unwrap();
    assert_eq!(before.unresolved_reservations, 2);
    assert_eq!(before.last_sequence, 3);

    let refused = reconcile_effects(&fixture.store, &fixture.job, &stale.digest).await;
    assert!(matches!(refused, Err(ReconcileError::StalePlan)));
    let after = fixture
        .store
        .coding_recovery_status(&fixture.job)
        .await
        .unwrap();
    assert_eq!(
        after.unresolved_reservations,
        before.unresolved_reservations
    );
    assert_eq!(after.last_sequence, before.last_sequence);

    // Positive control: the current digest is accepted and reconciles
    // exactly the two effects the stale digest missed.
    let current = plan_reconciliation(&fixture.store, &fixture.job)
        .await
        .unwrap();
    assert_ne!(current.digest, stale.digest);
    let report = reconcile_effects(&fixture.store, &fixture.job, &current.digest)
        .await
        .unwrap();
    assert_eq!(
        report.reconciled,
        vec![NonZeroU64::new(2).unwrap(), NonZeroU64::new(3).unwrap()]
    );
    fixture.close().await;
}

#[tokio::test]
async fn reconciling_twice_is_a_no_op_and_a_terminal_job_is_not_reconcilable() {
    let fixture = Fixture::new("reconcile-twice").await;
    let owner_limits = limits(8, 8, 8, 2, 1, Duration::from_secs(30));
    let ledger = fixture.ledger(owner_limits).await.unwrap();
    let _permit = ledger.root_view().admit_model_request().await.unwrap();
    fixture.close_store().await;
    drop(ledger);

    let store = fixture.reopen();
    let plan = plan_reconciliation(&store, &fixture.job).await.unwrap();
    let report = reconcile_effects(&store, &fixture.job, &plan.digest)
        .await
        .unwrap();
    assert_eq!(report.reconciled.len(), 1);

    let again = plan_reconciliation(&store, &fixture.job).await.unwrap();
    assert!(again.effects.is_empty());
    assert_ne!(again.digest, plan.digest);
    let empty = reconcile_effects(&store, &fixture.job, &again.digest)
        .await
        .unwrap();
    assert!(empty.reconciled.is_empty());

    // A terminal job is refused: append Completed through the store, then
    // both planning and reconciling fail closed (the same planning call
    // succeeded while the job was Running above — the positive control).
    let completed = job_event(
        &fixture.job,
        4,
        CodingEventKind::StateChanged {
            state: JobState::Completed,
        },
    );
    store.append_coding_event(completed).await.unwrap();
    let planned = plan_reconciliation(&store, &fixture.job).await;
    assert!(matches!(planned, Err(ReconcileError::NotReconcilable)));
    let written = reconcile_effects(&store, &fixture.job, &again.digest).await;
    assert!(matches!(written, Err(ReconcileError::NotReconcilable)));
    fixture.close().await;
}

#[tokio::test]
async fn a_job_with_an_active_child_is_not_reconcilable() {
    let fixture = Fixture::new("reconcile-child").await;
    let authority = fixture
        .add_child("agent-reconcile-c", "invocation-reconcile-c")
        .await;
    let actor = (authority.agent_id(), authority.invocation_id());
    let running = child_event(
        &fixture.job,
        actor,
        2,
        CodingEventKind::StateChanged {
            state: JobState::Running,
        },
    );
    fixture.store.append_coding_event(running).await.unwrap();
    let status = fixture
        .store
        .coding_recovery_status(&fixture.job)
        .await
        .unwrap();
    assert_eq!(status.active_children, 1);

    let refused = plan_reconciliation(&fixture.store, &fixture.job).await;
    assert!(matches!(refused, Err(ReconcileError::ChildrenActive)));

    // Positive control: once the child is no longer active, the same job
    // plans (an empty plan — nothing unresolved) and reconciles.
    let cancelled = child_event(
        &fixture.job,
        actor,
        3,
        CodingEventKind::StateChanged {
            state: JobState::Cancelled,
        },
    );
    fixture.store.append_coding_event(cancelled).await.unwrap();
    let plan = plan_reconciliation(&fixture.store, &fixture.job)
        .await
        .unwrap();
    assert!(plan.effects.is_empty());
    let report = reconcile_effects(&fixture.store, &fixture.job, &plan.digest)
        .await
        .unwrap();
    assert!(report.reconciled.is_empty());
    fixture.close().await;
}
