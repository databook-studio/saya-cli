use super::support::Fixture;
use saya_store::{NewCodingChild, SqliteStateStore, StoreError};
use saya_types::{
    AgentId, CheckoutOperationState, InvocationId, ManagedCheckoutId, RepositoryMutationAction,
};
use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};
use std::num::NonZeroU64;

#[tokio::test]
async fn operation_intent_requires_the_stable_checkout_owner() {
    let fixture = Fixture::new("owner-binding").await;
    let owner = AgentId::parse("agent-r14-owner").unwrap();
    fixture
        .store
        .create_coding_child(NewCodingChild {
            job_id: fixture.job.clone(),
            agent_id: owner.clone(),
            current_invocation_id: InvocationId::parse("invocation-r14-owner").unwrap(),
        })
        .await
        .unwrap();
    fixture
        .store
        .record_checkout_create_intent(fixture.create_event(
            2,
            "checkout-r14-ownerbound",
            fixture.repository.clone(),
            fixture.snapshot.clone(),
            Some(owner.clone()),
        ))
        .await
        .unwrap();
    let completed = fixture.operation_event(
        3,
        2,
        "checkout-r14-ownerbound",
        RepositoryMutationAction::CreateCheckout,
        CheckoutOperationState::Completed,
        Some(owner.clone()),
    );
    fixture
        .store
        .record_checkout_operation_outcome(completed)
        .await
        .unwrap();

    let wrong_actor = fixture.operation_event(
        4,
        4,
        "checkout-r14-ownerbound",
        RepositoryMutationAction::IntegrateCandidate,
        CheckoutOperationState::Intent,
        Some(AgentId::parse("agent-r14-other").unwrap()),
    );
    assert_eq!(
        fixture
            .store
            .record_checkout_operation_intent(wrong_actor)
            .await
            .unwrap_err(),
        StoreError::Conflict,
    );
    let rightful_actor = fixture.operation_event(
        4,
        4,
        "checkout-r14-ownerbound",
        RepositoryMutationAction::IntegrateCandidate,
        CheckoutOperationState::Intent,
        Some(owner),
    );
    fixture
        .store
        .record_checkout_operation_intent(rightful_actor)
        .await
        .unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn operation_reads_reject_owner_mismatched_intent_and_outcome_events() {
    let fixture = Fixture::new("owner-corrupt-read").await;
    fixture.create(2, "checkout-r14-intent-corrupt").await;
    fixture
        .complete_create(3, 2, "checkout-r14-intent-corrupt")
        .await;
    fixture
        .store
        .record_checkout_operation_intent(fixture.operation_event(
            4,
            4,
            "checkout-r14-intent-corrupt",
            RepositoryMutationAction::IntegrateCandidate,
            CheckoutOperationState::Intent,
            None,
        ))
        .await
        .unwrap();
    fixture
        .store
        .record_checkout_operation_outcome(fixture.operation_event(
            5,
            4,
            "checkout-r14-intent-corrupt",
            RepositoryMutationAction::IntegrateCandidate,
            CheckoutOperationState::Completed,
            None,
        ))
        .await
        .unwrap();
    fixture.create(6, "checkout-r14-outcome-corrupt").await;
    fixture
        .complete_create(7, 6, "checkout-r14-outcome-corrupt")
        .await;
    fixture
        .store
        .record_checkout_operation_intent(fixture.operation_event(
            8,
            8,
            "checkout-r14-outcome-corrupt",
            RepositoryMutationAction::ApplyBack,
            CheckoutOperationState::Intent,
            None,
        ))
        .await
        .unwrap();
    fixture
        .store
        .record_checkout_operation_outcome(fixture.operation_event(
            9,
            8,
            "checkout-r14-outcome-corrupt",
            RepositoryMutationAction::ApplyBack,
            CheckoutOperationState::Completed,
            None,
        ))
        .await
        .unwrap();

    let database = fixture.database.clone();
    let root = fixture.root.clone();
    let job = fixture.job.clone();
    let forged = AgentId::parse("agent-r14-forged").unwrap();
    let bad_intent = fixture
        .operation_event(
            4,
            4,
            "checkout-r14-intent-corrupt",
            RepositoryMutationAction::IntegrateCandidate,
            CheckoutOperationState::Intent,
            Some(forged.clone()),
        )
        .to_json_bytes()
        .unwrap();
    let bad_outcome = fixture
        .operation_event(
            9,
            8,
            "checkout-r14-outcome-corrupt",
            RepositoryMutationAction::ApplyBack,
            CheckoutOperationState::Completed,
            Some(forged),
        )
        .to_json_bytes()
        .unwrap();
    fixture.store.close().await;
    let pool = raw_pool(&database).await;
    for (sequence, json) in [(4_i64, bad_intent), (9_i64, bad_outcome)] {
        sqlx::query("UPDATE coding_events SET event_json=? WHERE job_id=? AND sequence=?")
            .bind(String::from_utf8(json).unwrap())
            .bind(job.as_str())
            .bind(sequence)
            .execute(&pool)
            .await
            .unwrap();
    }
    pool.close().await;

    let reopened = SqliteStateStore::new(&database);
    for (checkout_name, operation_sequence) in [
        ("checkout-r14-intent-corrupt", 4_u64),
        ("checkout-r14-outcome-corrupt", 8_u64),
    ] {
        let checkout = ManagedCheckoutId::parse(checkout_name).unwrap();
        assert_eq!(
            reopened
                .get_checkout_operation(
                    &job,
                    &checkout,
                    NonZeroU64::new(operation_sequence).unwrap(),
                )
                .await
                .unwrap_err(),
            StoreError::Invalid,
        );
        assert_eq!(
            reopened
                .list_checkout_operations(&job, &checkout, None, 10)
                .await
                .unwrap_err(),
            StoreError::Invalid,
        );
        assert_eq!(
            reopened
                .get_managed_checkout(&job, &checkout)
                .await
                .unwrap_err(),
            StoreError::Invalid,
        );
    }
    assert_eq!(
        reopened
            .list_managed_checkouts(&job, None, 10)
            .await
            .unwrap_err(),
        StoreError::Invalid,
    );
    reopened.close().await;
    let _ = std::fs::remove_dir_all(root);
}

async fn raw_pool(database: &std::path::Path) -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(database))
        .await
        .unwrap()
}
