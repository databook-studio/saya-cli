//! The CLI surface of the reconcile bound: more than 256 unresolved effects
//! refuse as a typed reconciliation refusal and write nothing.

use super::support::Fixture;
use saya_agent::CancellationToken;
use saya_cli::{
    CodingAuthorization, CodingCommand, CodingError, CodingJobRequest, dispatch_coding,
};
use saya_types::{
    BudgetCharge, BudgetDimension, BudgetReservationV1, CodingEventKind, CodingEventV1, JobId,
};
use std::num::NonZeroU64;

/// A job the application created and then crashed, left with `count`
/// open reservations appended in exact stream order.
async fn crashed_job_with_effects(fixture: &Fixture, job: &str, count: u64) -> JobId {
    let request = CodingJobRequest::new(
        JobId::parse(job).unwrap(),
        "inspect only".into(),
        vec![],
        vec![],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let runtime = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    let id = runtime.job_id().clone();
    drop(runtime);
    let status = fixture.store.coding_recovery_status(&id).await.unwrap();
    let authority = status
        .authority
        .expect("a created job has runtime authority");
    for offset in 0..count {
        let sequence = status.last_sequence + 1 + offset;
        let reservation = BudgetReservationV1::new(
            id.clone(),
            NonZeroU64::new(sequence).unwrap(),
            None,
            None,
            vec![
                BudgetCharge::new(
                    BudgetDimension::LogicalRequests,
                    None,
                    None,
                    NonZeroU64::new(1).unwrap(),
                )
                .unwrap(),
            ],
        )
        .unwrap();
        let reserved = NonZeroU64::new(sequence).unwrap();
        let event = CodingEventV1::new(
            reserved,
            id.clone(),
            None,
            None,
            CodingEventKind::BudgetReserved {
                reservation: Box::new(reservation),
            },
        )
        .unwrap();
        fixture
            .store
            .reserve_coding_budget_fenced(event, Some(&authority))
            .await
            .unwrap();
    }
    id
}

#[tokio::test]
async fn more_than_256_unresolved_effects_refuse_as_a_reconciliation_refusal() {
    let fixture = Fixture::new("reconcile-bounds-cli").await;
    let job = crashed_job_with_effects(&fixture, "job-reconcile-bounds", 257).await;
    let before = fixture.store.coding_recovery_status(&job).await.unwrap();
    assert_eq!(before.unresolved_reservations, 257);

    let refused = dispatch_coding(
        &fixture.application,
        CodingCommand::Reconcile {
            id: job.as_str().into(),
            plan_sha256: None,
            approve: false,
        },
    )
    .await
    .unwrap_err();
    assert!(
        matches!(
            &refused,
            CodingError::ReconciliationRefused(text)
                if text == "more than 256 unresolved effects"
        ),
        "the bound refuses as a typed reconciliation refusal: {refused:?}"
    );

    let after = fixture.store.coding_recovery_status(&job).await.unwrap();
    assert_eq!(
        after.last_sequence, before.last_sequence,
        "the refusal wrote nothing"
    );
    assert_eq!(after.unresolved_reservations, 257);
}
