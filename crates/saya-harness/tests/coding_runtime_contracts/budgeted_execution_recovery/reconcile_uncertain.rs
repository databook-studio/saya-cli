//! A model request Saya gave up on settles with unknown usage: it needs no
//! reconciliation before resume, its charge stays counted and its slot is
//! free. A request that never settled (the process died) is a different
//! residue and still needs reconciling, with its charge and slot consumed.

use std::time::Duration;

use saya_agent::{EffectAdmission, EffectOutcome, EffectSettlement};
use saya_harness::coding::budget::UsageCoverage;
use saya_harness::coding::coordinator::{
    ResumeError, plan_reconciliation, reconcile_effects, resume_job,
};
use saya_store::CodingRuntimeAuthority;
use saya_types::{BudgetEffectState, Sha256Digest};

use super::support::{Fixture, limits};

fn authority() -> CodingRuntimeAuthority {
    CodingRuntimeAuthority {
        epoch: 2,
        policy_digest: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
    }
}

#[tokio::test]
async fn an_unknown_model_request_resumes_without_reconcile_and_frees_its_slot() {
    let fixture = Fixture::new("reconcile-unknown-slot").await;
    let owner_limits = limits(8, 8, 8, 2, 1, Duration::from_secs(30));
    let ledger = fixture.ledger(owner_limits).await.unwrap();
    let root = ledger.root_view();
    let permit = root.admit_model_request().await.unwrap();
    root.settle(
        permit,
        EffectSettlement::ModelRequest {
            outcome: EffectOutcome::Unknown,
            reported_usage: None,
            usage_complete: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(ledger.snapshot().await.model_slots_active, 0);
    drop(root);
    fixture.close_store().await;
    drop(ledger);

    let store = fixture.reopen();
    let reservations = store
        .list_coding_budget_reservations(&fixture.job, None, 100)
        .await
        .unwrap();
    assert!(reservations.items.iter().all(|row| {
        row.settlement
            .as_ref()
            .is_some_and(|s| s.effect_state() == BudgetEffectState::Settled)
    }));
    let plan = plan_reconciliation(&store, &fixture.job).await.unwrap();
    assert!(plan.effects.is_empty(), "nothing is left to reconcile");

    let resumed = resume_job(
        store.clone(),
        fixture.job.clone(),
        owner_limits,
        authority(),
    )
    .await
    .expect("an unknown model request resumes without reconciliation");
    let snapshot = resumed.snapshot().await;
    assert_eq!(snapshot.logical_requests, 1, "the request stays charged");
    assert_eq!(snapshot.unknown_logical_requests, 1);
    assert_eq!(snapshot.usage_coverage, UsageCoverage::Unknown);
    assert_eq!(snapshot.model_slots_active, 0, "its slot is free");
    fixture.close().await;
}

#[tokio::test]
async fn an_unsettled_model_request_still_resumes_only_after_reconcile_and_stays_charged() {
    let fixture = Fixture::new("reconcile-unsettled-slot").await;
    let owner_limits = limits(8, 8, 8, 2, 1, Duration::from_secs(30));
    let ledger = fixture.ledger(owner_limits).await.unwrap();
    let root = ledger.root_view();
    let _never_settled = root.admit_model_request().await.unwrap();
    drop(root);
    fixture.close_store().await;
    drop(ledger);

    let store = fixture.reopen();
    // Control: the process died mid-request, so it is unresolved.
    assert!(matches!(
        resume_job(
            store.clone(),
            fixture.job.clone(),
            owner_limits,
            authority()
        )
        .await,
        Err(ResumeError::ReconciliationRequired(_))
    ));
    let plan = plan_reconciliation(&store, &fixture.job).await.unwrap();
    assert_eq!(plan.effects.len(), 1);
    reconcile_effects(&store, &fixture.job, &plan.digest)
        .await
        .unwrap();

    let resumed = resume_job(
        store.clone(),
        fixture.job.clone(),
        owner_limits,
        authority(),
    )
    .await
    .expect("a reconciled open model request resumes");
    let snapshot = resumed.snapshot().await;
    assert_eq!(snapshot.logical_requests, 1, "the request stays charged");
    assert_eq!(snapshot.model_slots_active, 1, "its slot stays consumed");
    fixture.close().await;
}
