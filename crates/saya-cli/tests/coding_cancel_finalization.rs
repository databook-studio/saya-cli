//! Durable cancellation stays observed while the root waits on a live helper.
//!
//! The root hands one piece to a helper through `helpers`, which is the whole
//! delegation surface: the call waits for the helper inside the tool, so
//! the root's model turn is over and the run is parked on the live child. A
//! durable cancel requested there must still stop the run.
#![cfg(unix)]
#[path = "coding_failure_reporting/held_provider.rs"]
mod provider;
#[path = "coding_application/support.rs"]
#[allow(dead_code)] // Shared real repository/application fixture.
mod support;
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingApplication, CodingAuthorization, CodingJobRequest};
use saya_types::{CodingBudgetMode, JobId};
use std::{sync::Arc, time::Duration};
#[tokio::test]
async fn durable_cancel_after_root_answer_interrupts_implicit_join() {
    let server = provider::Provider::new().await;
    let fixture =
        support::Fixture::with_endpoint("implicit-join-cancel", Some(&server.endpoint)).await;
    let mut runtime = fixture.runtime.clone();
    runtime.resolved.limits.job.budget_mode = CodingBudgetMode::Uncapped;
    runtime.resolved.ai.timeout_seconds = 30;
    runtime.resolved.ai.idle_timeout_seconds = 30;
    let application = CodingApplication::open(
        runtime,
        fixture.store.clone(),
        &fixture.repository,
        &fixture.root.join("uncapped"),
        &fixture.git,
        &fixture.program_dir,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let request = CodingJobRequest::new(
        JobId::parse("implicit-join-cancel").unwrap(),
        "hand one piece to a helper and wait for it".into(),
        vec!["child.txt".into()],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&application.propose(&request).await.unwrap()).unwrap();
    let job = application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    let running = Arc::clone(&job);
    let mut run = tokio::spawn(async move { running.run(&NoopEventSink).await });
    // A hang guard, not the test: the direct root's own working checkout and
    // the helper are created before the child's first request.
    tokio::time::timeout(Duration::from_secs(30), server.child_entered.notified())
        .await
        .unwrap();
    // The helper's request has arrived and is never answered, so the root's
    // `helpers` call is parked on the live child.
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(!run.is_finished());
    assert!(
        fixture
            .store
            .request_coding_cancel(job.job_id())
            .await
            .unwrap()
    );
    let result = tokio::time::timeout(Duration::from_secs(2), &mut run).await;
    if result.is_err() {
        job.cancel();
        let _ = tokio::time::timeout(Duration::from_secs(10), &mut run).await;
        panic!("durable cancellation stopped being observed after root completion");
    }
    let _ = result.unwrap().unwrap();
    assert_eq!(
        application.status(job.job_id()).await.unwrap()["state"],
        "cancelled"
    );
    let recovery = fixture
        .store
        .coding_recovery_status(job.job_id())
        .await
        .unwrap();
    let reservations = fixture
        .store
        .list_coding_budget_reservations(job.job_id(), None, 100)
        .await
        .unwrap();
    let unknown = reservations
        .items
        .iter()
        .filter(|record| {
            record.settlement.as_ref().is_some_and(|settlement| {
                settlement
                    .observations()
                    .iter()
                    .any(|item| item.value() == saya_types::BudgetObservationValue::Unknown)
            })
        })
        .collect::<Vec<_>>();
    // The held HTTP request was sent but never answered. Local joining cannot
    // refund it, but it can only have lost its usage: the request and its
    // send settle with unknown usage, leave nothing unresolved, and need no
    // reconciliation.
    assert_eq!(recovery.unresolved_reservations, 0);
    assert_eq!(recovery.active_children, 0);
    assert_eq!(unknown.len(), 2);
    let children = fixture
        .store
        .list_coding_children(job.job_id(), None, 10)
        .await
        .unwrap();
    assert_eq!(children.items.len(), 1);
    for record in &unknown {
        assert_eq!(
            record.reservation.agent_id(),
            Some(&children.items[0].agent_id)
        );
        assert_eq!(
            record.reservation.invocation_id(),
            Some(&children.items[0].current_invocation_id)
        );
        assert_eq!(
            record.settlement.as_ref().unwrap().effect_state(),
            saya_types::BudgetEffectState::Settled
        );
        assert!(
            record
                .reservation
                .charges()
                .iter()
                .all(|charge| charge.reserved().get() == 1)
        );
    }
    let dimensions = unknown
        .iter()
        .map(|record| {
            record
                .reservation
                .charges()
                .iter()
                .map(|charge| charge.dimension())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        dimensions,
        vec![
            vec![
                saya_types::BudgetDimension::LogicalRequests,
                saya_types::BudgetDimension::ModelRequestSlots
            ],
            vec![saya_types::BudgetDimension::TransportAttempts]
        ]
    );
    let proposal = application.propose_resume(job.job_id()).await.unwrap();
    let approval = CodingAuthorization::scoped(&proposal).unwrap();
    assert!(
        application
            .resume(job.job_id(), &approval, CancellationToken::new())
            .await
            .is_err(),
        "a cancelled job stays terminal, with or without unresolved effects"
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
    assert!(!fixture.repository.join("child.txt").exists());
    assert_eq!(
        std::fs::read(fixture.repository.join("tracked.txt")).unwrap(),
        b"original\n"
    );
}
