use super::support::Fixture;
use saya_store::{CodingEventAppend, StoreError};
use saya_types::{AgentId, CheckoutOperationState, ManagedCheckoutId, RepositoryId, SnapshotId};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::num::NonZeroU64;

#[tokio::test]
async fn invalid_create_intents_roll_back_their_event_and_allow_retry() {
    let fixture = Fixture::new("invalid-create").await;
    let wrong_repository = fixture.create_event(
        2,
        "checkout-r14-wrongrepo",
        RepositoryId::parse("repo-other").unwrap(),
        fixture.snapshot.clone(),
        None,
    );
    assert_eq!(
        fixture
            .store
            .record_checkout_create_intent(wrong_repository)
            .await
            .unwrap_err(),
        StoreError::Conflict
    );
    let wrong_snapshot = fixture.create_event(
        2,
        "checkout-r14-wrongsnapshot",
        fixture.repository.clone(),
        SnapshotId::parse("snapshot-other").unwrap(),
        None,
    );
    assert_eq!(
        fixture
            .store
            .record_checkout_create_intent(wrong_snapshot)
            .await
            .unwrap_err(),
        StoreError::Conflict
    );
    let missing_owner = fixture.create_event(
        2,
        "checkout-r14-owner",
        fixture.repository.clone(),
        fixture.snapshot.clone(),
        Some(AgentId::parse("agent-r14-missing").unwrap()),
    );
    assert_eq!(
        fixture
            .store
            .record_checkout_create_intent(missing_owner)
            .await
            .unwrap_err(),
        StoreError::NotFound
    );
    assert_eq!(
        fixture
            .store
            .list_coding_events(&fixture.job, None, 100)
            .await
            .unwrap()
            .events
            .len(),
        1
    );

    let valid = fixture.create_event(
        2,
        "checkout-r14-valid",
        fixture.repository.clone(),
        fixture.snapshot.clone(),
        None,
    );
    assert_eq!(
        fixture
            .store
            .record_checkout_create_intent(valid)
            .await
            .unwrap(),
        CodingEventAppend::Appended(NonZeroU64::new(2).unwrap())
    );
    fixture.close().await;
}

#[tokio::test]
async fn trigger_failure_rolls_back_event_ownership_and_operation_then_retry_succeeds() {
    let fixture = Fixture::new("trigger").await;
    let event = fixture.create_event(
        2,
        "checkout-r14-trigger",
        fixture.repository.clone(),
        fixture.snapshot.clone(),
        None,
    );
    let pool = pool(&fixture.database).await;
    sqlx::query("CREATE TRIGGER refuse_checkout BEFORE INSERT ON coding_checkout_operations BEGIN SELECT RAISE(ABORT, 'test rejection'); END")
        .execute(&pool).await.unwrap();
    assert_eq!(
        fixture
            .store
            .record_checkout_create_intent(event.clone())
            .await
            .unwrap_err(),
        StoreError::Unavailable
    );
    assert_eq!(
        fixture
            .store
            .list_coding_events(&fixture.job, None, 100)
            .await
            .unwrap()
            .events
            .len(),
        1
    );
    assert_eq!(
        fixture
            .store
            .get_managed_checkout(
                &fixture.job,
                &ManagedCheckoutId::parse("checkout-r14-trigger").unwrap(),
            )
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        fixture
            .store
            .get_checkout_operation(
                &fixture.job,
                &ManagedCheckoutId::parse("checkout-r14-trigger").unwrap(),
                NonZeroU64::new(2).unwrap(),
            )
            .await
            .unwrap(),
        None
    );

    sqlx::query("DROP TRIGGER refuse_checkout")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        fixture
            .store
            .record_checkout_create_intent(event)
            .await
            .unwrap(),
        CodingEventAppend::Appended(NonZeroU64::new(2).unwrap())
    );
    assert_eq!(
        fixture
            .store
            .get_checkout_operation(
                &fixture.job,
                &ManagedCheckoutId::parse("checkout-r14-trigger").unwrap(),
                NonZeroU64::new(2).unwrap(),
            )
            .await
            .unwrap()
            .unwrap()
            .state(),
        CheckoutOperationState::Intent
    );
    pool.close().await;
    fixture.close().await;
}

async fn pool(database: &std::path::Path) -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(database))
        .await
        .unwrap()
}
