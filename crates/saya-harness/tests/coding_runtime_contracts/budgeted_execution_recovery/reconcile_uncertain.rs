//! A model request whose outcome was observed Unknown settles `Uncertain` with
//! an Unknown slot observation. That is the commonest crash residue, and it
//! must be resumable once reconciled (its charge and slot stay consumed).

use std::time::Duration;

use saya_agent::{EffectAdmission, EffectOutcome, EffectSettlement};
use saya_harness::coding::budget::{BudgetLedgerError, JobBudgetLedger};
use saya_harness::coding::coordinator::{plan_reconciliation, reconcile_effects, resume_job};
use saya_store::CodingRuntimeAuthority;
use saya_types::{BudgetEffectState, Sha256Digest};

use super::support::{Fixture, limits};

#[tokio::test]
async fn an_uncertain_model_request_resumes_only_after_reconcile_and_stays_charged() {
    let fixture = Fixture::new("reconcile-uncertain-slot").await;
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
            .is_some_and(|s| s.effect_state() == BudgetEffectState::Uncertain)
    }));
    // Control: before reconcile the uncertain slot observation is not foldable.
    assert!(matches!(
        JobBudgetLedger::resume(store.clone(), fixture.job.clone(), owner_limits).await,
        Err(BudgetLedgerError::ResumeRecordCorrupt)
    ));

    let plan = plan_reconciliation(&store, &fixture.job).await.unwrap();
    assert!(plan.effects.iter().all(|effect| effect.settled_uncertain));
    reconcile_effects(&store, &fixture.job, &plan.digest)
        .await
        .unwrap();

    let authority = CodingRuntimeAuthority {
        epoch: 2,
        policy_digest: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
    };
    let resumed = resume_job(store.clone(), fixture.job.clone(), owner_limits, authority)
        .await
        .expect("a reconciled uncertain model request resumes");
    let snapshot = resumed.snapshot().await;
    assert_eq!(snapshot.logical_requests, 1, "the request stays charged");
    assert_eq!(snapshot.model_slots_active, 1, "its slot stays consumed");
    fixture.close().await;
}
