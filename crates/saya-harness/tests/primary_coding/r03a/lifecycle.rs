use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use saya_agent::{CancellationReason, CancellationToken};
use saya_harness::coding::coordinator::{ChildRunError, child_event_channel, run_child_budgeted};
use saya_types::{BudgetDimension, BudgetEffectState, BudgetObservationValue};

use super::{
    child_support::{FactoryStats, OpenAiFactory},
    fixture::{Fixture, limits},
    openai_server::{ANSWER_RESPONSE, LocalOpenAi},
};

fn factory(server: &LocalOpenAi, fixture: &Fixture, stats: Arc<FactoryStats>) -> OpenAiFactory {
    OpenAiFactory {
        endpoint: server.endpoint.clone(),
        database: fixture.database.clone(),
        job: fixture.job.clone(),
        stats,
        cancel_on_build: None,
        retry_delays: vec![Duration::from_millis(5)],
    }
}

#[tokio::test]
async fn dispatched_cancellation_keeps_unknown_usage_and_count_reservations() {
    let fixture = Fixture::new("dispatched-cancel").await;
    let authority = fixture.add_child("agent-cancel", "invocation-cancel").await;
    let mut server = LocalOpenAi::new().await;
    let ledger = fixture
        .ledger(limits(2, 2, 1, 1, 1, Duration::from_secs(5)))
        .await
        .unwrap();
    let spec = fixture.spec(authority, "cancel after send", 2, 1);
    let (sender, _receiver) = child_event_channel(&spec);
    let stats = Arc::new(FactoryStats::default());
    let child_factory = factory(&server, &fixture, Arc::clone(&stats));
    let cancellation = ledger.cancellation_token();
    let child_ledger = ledger.clone();
    let child_cancellation = cancellation.clone();
    let child = tokio::spawn(async move {
        run_child_budgeted(
            spec,
            &child_factory,
            sender,
            child_cancellation,
            &child_ledger,
        )
        .await
    });
    assert_eq!(server.request().await, Some(1));
    cancellation.cancel();
    let result = tokio::time::timeout(Duration::from_secs(2), child)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        result,
        Err(ChildRunError::Cancelled {
            reason: CancellationReason::User
        })
    ));
    stats.assert_invocation_cancellation(CancellationReason::User);
    let snapshot = ledger.snapshot().await;
    assert_eq!(snapshot.logical_requests, 1);
    assert_eq!(snapshot.transport_attempts, 1);
    assert!(snapshot.unknown_transport_attempts >= 1);
    assert!(snapshot.unknown_logical_requests >= 1);
    let durable = fixture
        .store
        .list_coding_budget_reservations(&fixture.job, None, 10)
        .await
        .unwrap();
    for dimension in [
        BudgetDimension::LogicalRequests,
        BudgetDimension::TransportAttempts,
    ] {
        let record = durable
            .items
            .iter()
            .find(|record| {
                record
                    .reservation
                    .charges()
                    .iter()
                    .any(|charge| charge.dimension() == dimension)
            })
            .unwrap_or_else(|| panic!("missing durable {dimension:?} reservation"));
        let settlement = record.settlement.as_ref().unwrap();
        assert_eq!(settlement.effect_state(), BudgetEffectState::Uncertain);
        assert!(
            settlement.observations().iter().any(|observation| {
                observation.dimension() == dimension
                    && observation.value() == BudgetObservationValue::Unknown
            }),
            "missing unknown {dimension:?} observation"
        );
    }
    server.respond_sse(ANSWER_RESPONSE);
    server.shutdown().await;
    fixture.close().await;
}

#[tokio::test]
async fn root_deadline_cancels_and_joins_a_child_with_an_open_provider_request() {
    let fixture = Fixture::new("deadline-child").await;
    let authority = fixture
        .add_child("agent-deadline", "invocation-deadline")
        .await;
    let mut server = LocalOpenAi::new().await;
    // The deadline outlasts the bounded dispatch wait below so preparation
    // cannot win the race; dispatch is asserted before the deadline outcome.
    let ledger = fixture
        .ledger(limits(2, 2, 1, 1, 1, Duration::from_secs(3)))
        .await
        .unwrap();
    let spec = fixture.spec(authority, "wait forever", 2, 1);
    let (sender, _receiver) = child_event_channel(&spec);
    let stats = Arc::new(FactoryStats::default());
    let child_factory = factory(&server, &fixture, Arc::clone(&stats));
    let child_ledger = ledger.clone();
    let caller_cancellation = CancellationToken::new();
    let child_cancellation = caller_cancellation.clone();
    let child = tokio::spawn(async move {
        run_child_budgeted(
            spec,
            &child_factory,
            sender,
            child_cancellation,
            &child_ledger,
        )
        .await
    });
    assert_eq!(
        server.request_within(Duration::from_secs(2)).await,
        Some(1),
        "first provider request was not observed before the deadline"
    );
    let result = tokio::time::timeout(Duration::from_secs(20), child)
        .await
        .expect("child did not finish after the root deadline elapsed")
        .unwrap()
        .expect("a deadline-stopped child completes with its answering-budget output");
    // A controlled answering run surfaces a budget-dimension cancellation as a
    // truncated output, not a cancelled error: the answering admission maps a
    // Budget cancellation reason to Limit("answering budget stopped") (saya-agent
    // src/loop_runner/admission.rs cancellation_error), the turn converts a
    // cancelled receive into that limit and returns Ok(budget_output)
    // (src/loop_runner/turn.rs, and the top-of-turn check in
    // src/loop_runner/runner.rs), and the coordinator passes a non-User-reason Ok
    // through (src/coding/coordinator/api.rs run_child_inner). The deadline still
    // cancels and joins the child: the invocation and ledger tokens below carry
    // the ElapsedTime reason and the child slot drains.
    assert!(
        result.truncated,
        "the deadline stop surfaces as a truncated answering-budget output"
    );
    assert_eq!(result.answer, "");
    assert!(result.events.iter().any(|event| matches!(
        event,
        saya_agent::AgentEvent::AnsweringBudgetStopped {
            dimension: saya_agent::BudgetDimension::ElapsedTime
        } | _
    )));
    stats.assert_invocation_cancellation(CancellationReason::Budget(
        saya_agent::BudgetDimension::ElapsedTime,
    ));
    assert_eq!(caller_cancellation.reason(), None);
    assert_eq!(
        ledger.cancellation_token().reason(),
        Some(CancellationReason::Budget(
            saya_agent::BudgetDimension::ElapsedTime
        ))
    );
    let snapshot = ledger.snapshot().await;
    assert!(!snapshot.poisoned);
    assert_eq!(snapshot.child_slots_active, 0);
    server.respond_sse(ANSWER_RESPONSE);
    server.shutdown().await;
    fixture.close().await;
}

#[tokio::test]
async fn cancellation_after_child_slot_but_before_model_dispatch_is_known_not_started() {
    let fixture = Fixture::new("predispatch-cancel").await;
    let authority = fixture
        .add_child("agent-pre-cancel", "invocation-pre-cancel")
        .await;
    let mut server = LocalOpenAi::new().await;
    let ledger = fixture
        .ledger(limits(2, 2, 1, 1, 1, Duration::from_secs(5)))
        .await
        .unwrap();
    let spec = fixture.spec(authority, "do not send", 2, 1);
    let (sender, _receiver) = child_event_channel(&spec);
    let cancellation = CancellationToken::new();
    let stats = Arc::new(FactoryStats::default());
    let mut child_factory = factory(&server, &fixture, Arc::clone(&stats));
    child_factory.cancel_on_build = Some(cancellation.clone());
    let child_ledger = ledger.clone();
    let child = tokio::spawn(async move {
        run_child_budgeted(spec, &child_factory, sender, cancellation, &child_ledger).await
    });
    let result = child.await.unwrap();
    assert!(matches!(
        result,
        Err(ChildRunError::Cancelled {
            reason: CancellationReason::User
        })
    ));
    assert_eq!(stats.builds.load(Ordering::SeqCst), 1);
    assert_eq!(server.count.load(Ordering::SeqCst), 0);
    let reservations = fixture
        .store
        .list_coding_budget_reservations(&fixture.job, None, 10)
        .await
        .unwrap();
    assert_eq!(reservations.items.len(), 1);
    let settlement = reservations.items[0].settlement.as_ref().unwrap();
    assert_eq!(settlement.effect_state(), BudgetEffectState::Settled);
    assert_eq!(
        settlement.observations()[0].value(),
        BudgetObservationValue::Measured(0)
    );
    server.shutdown().await;
    fixture.close().await;
}

#[tokio::test]
async fn racing_children_share_one_child_slot_and_only_the_winner_builds() {
    let fixture = Fixture::new("child-slot-race").await;
    let first_authority = fixture
        .add_child("agent-slot-one", "invocation-slot-one")
        .await;
    let second_authority = fixture
        .add_child("agent-slot-two", "invocation-slot-two")
        .await;
    let mut server = LocalOpenAi::new().await;
    let ledger = fixture
        .ledger(limits(3, 3, 1, 1, 1, Duration::from_secs(5)))
        .await
        .unwrap();
    let first_spec = fixture.spec(first_authority, "child one", 2, 1);
    let second_spec = fixture.spec(second_authority, "child two", 2, 1);
    let (first_sender, _first_receiver) = child_event_channel(&first_spec);
    let (second_sender, _second_receiver) = child_event_channel(&second_spec);
    let stats = Arc::new(FactoryStats::default());
    let first_factory = factory(&server, &fixture, Arc::clone(&stats));
    let second_factory = factory(&server, &fixture, Arc::clone(&stats));
    let first_ledger = ledger.clone();
    let second_ledger = ledger.clone();
    let first = tokio::spawn(async move {
        run_child_budgeted(
            first_spec,
            &first_factory,
            first_sender,
            CancellationToken::new(),
            &first_ledger,
        )
        .await
    });
    let second = tokio::spawn(async move {
        run_child_budgeted(
            second_spec,
            &second_factory,
            second_sender,
            CancellationToken::new(),
            &second_ledger,
        )
        .await
    });
    assert_eq!(server.request().await, Some(1));
    server.respond_sse(ANSWER_RESPONSE);
    let results = [first.await.unwrap(), second.await.unwrap()];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(ChildRunError::BudgetDenied)))
            .count(),
        1
    );
    assert_eq!(stats.builds.load(Ordering::SeqCst), 1);
    assert_eq!(server.count.load(Ordering::SeqCst), 1);
    let reservations = fixture
        .store
        .list_coding_budget_reservations(&fixture.job, None, 10)
        .await
        .unwrap();
    assert_eq!(
        reservations
            .items
            .iter()
            .filter(|row| row
                .reservation
                .charges()
                .iter()
                .any(|charge| charge.dimension() == BudgetDimension::ChildSlots))
            .count(),
        1
    );
    server.shutdown().await;
    fixture.close().await;
}
