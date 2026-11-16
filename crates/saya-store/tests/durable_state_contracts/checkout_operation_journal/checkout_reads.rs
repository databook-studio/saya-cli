use super::support::Fixture;
use saya_store::{SqliteStateStore, StoreError};
use saya_types::{
    CheckoutOperationState, ManagedCheckoutId, ManagedCheckoutLifecycle, RepositoryMutationAction,
};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::num::NonZeroU64;

#[tokio::test]
async fn checkout_and_operation_pages_are_keyset_bounded() {
    let fixture = Fixture::new("pages").await;
    fixture.create(2, "checkout-r14-a").await;
    fixture.create(3, "checkout-r14-b").await;
    fixture.complete_create(4, 2, "checkout-r14-a").await;
    let followup = fixture.operation_event(
        5,
        5,
        "checkout-r14-a",
        RepositoryMutationAction::IntegrateCandidate,
        CheckoutOperationState::Intent,
        None,
    );
    fixture
        .store
        .record_checkout_operation_intent(followup)
        .await
        .unwrap();
    let outcome = fixture.operation_event(
        6,
        5,
        "checkout-r14-a",
        RepositoryMutationAction::IntegrateCandidate,
        CheckoutOperationState::Completed,
        None,
    );
    fixture
        .store
        .record_checkout_operation_outcome(outcome)
        .await
        .unwrap();
    let first = fixture
        .store
        .list_managed_checkouts(&fixture.job, None, 1)
        .await
        .unwrap();
    assert_eq!(first.items.len(), 1);
    assert_eq!(
        first.items[0].current_lifecycle,
        ManagedCheckoutLifecycle::Retained
    );
    assert_eq!(
        first.next_cursor,
        Some(ManagedCheckoutId::parse("checkout-r14-a").unwrap())
    );
    let second = fixture
        .store
        .list_managed_checkouts(&fixture.job, first.next_cursor.as_ref(), 1)
        .await
        .unwrap();
    assert_eq!(second.items.len(), 1);
    assert_eq!(
        second.items[0].ownership.managed_checkout_id().as_str(),
        "checkout-r14-b"
    );
    assert_eq!(second.next_cursor, None);

    let first_operations = fixture
        .store
        .list_checkout_operations(
            &fixture.job,
            &ManagedCheckoutId::parse("checkout-r14-a").unwrap(),
            None,
            1,
        )
        .await
        .unwrap();
    assert_eq!(
        first_operations.items[0].operation_sequence(),
        NonZeroU64::new(2).unwrap()
    );
    assert_eq!(first_operations.next_cursor, NonZeroU64::new(2));
    let second_operations = fixture
        .store
        .list_checkout_operations(
            &fixture.job,
            &ManagedCheckoutId::parse("checkout-r14-a").unwrap(),
            first_operations.next_cursor,
            1,
        )
        .await
        .unwrap();
    assert_eq!(
        second_operations.items[0].operation_sequence(),
        NonZeroU64::new(5).unwrap()
    );
    assert_eq!(second_operations.next_cursor, None);
    assert_eq!(
        fixture
            .store
            .list_managed_checkouts(&fixture.job, None, 0)
            .await
            .unwrap_err(),
        StoreError::Invalid
    );
    assert_eq!(
        fixture
            .store
            .list_checkout_operations(
                &fixture.job,
                &ManagedCheckoutId::parse("checkout-r14-a").unwrap(),
                None,
                101,
            )
            .await
            .unwrap_err(),
        StoreError::LimitExceeded
    );
    fixture.close().await;
}

#[tokio::test]
async fn checkout_reads_refuse_future_and_mismatched_event_payloads() {
    let fixture = Fixture::new("corrupt-read").await;
    fixture.create(2, "checkout-r14-corrupt").await;
    fixture.store.close().await;
    let pool = pool(&fixture.database).await;
    let json: String =
        sqlx::query_scalar("SELECT event_json FROM coding_events WHERE job_id=? AND sequence=2")
            .bind(fixture.job.as_str())
            .fetch_one(&pool)
            .await
            .unwrap();
    let future = json.replacen("\"version\":1", "\"version\":99", 1);
    sqlx::query("UPDATE coding_events SET event_json=? WHERE job_id=? AND sequence=2")
        .bind(future)
        .bind(fixture.job.as_str())
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let reopened = SqliteStateStore::new(&fixture.database);
    assert_eq!(
        reopened
            .get_managed_checkout(
                &fixture.job,
                &ManagedCheckoutId::parse("checkout-r14-corrupt").unwrap(),
            )
            .await
            .unwrap_err(),
        StoreError::VersionUnsupported,
    );
    reopened.close().await;
    let _ = std::fs::remove_dir_all(fixture.root);
}

#[tokio::test]
async fn operation_projection_action_mismatch_fails_closed() {
    let fixture = Fixture::new("action-corrupt").await;
    fixture.create(2, "checkout-r14-action").await;
    fixture.store.close().await;
    let pool = pool(&fixture.database).await;
    sqlx::query("UPDATE coding_checkout_operations SET action='apply_back' WHERE job_id=? AND operation_sequence=2")
        .bind(fixture.job.as_str())
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let reopened = SqliteStateStore::new(&fixture.database);
    assert_eq!(
        reopened
            .get_checkout_operation(
                &fixture.job,
                &ManagedCheckoutId::parse("checkout-r14-action").unwrap(),
                NonZeroU64::new(2).unwrap(),
            )
            .await
            .unwrap_err(),
        StoreError::Invalid,
    );
    reopened.close().await;
    let _ = std::fs::remove_dir_all(fixture.root);
}

#[tokio::test]
async fn checkout_reads_refuse_a_missing_referenced_event() {
    let fixture = Fixture::new("missing-event").await;
    fixture.create(2, "checkout-r14-missing").await;
    fixture.store.close().await;
    let pool = pool(&fixture.database).await;
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM coding_events WHERE job_id=? AND sequence=2")
        .bind(fixture.job.as_str())
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let reopened = SqliteStateStore::new(&fixture.database);
    assert_eq!(
        reopened
            .get_managed_checkout(
                &fixture.job,
                &ManagedCheckoutId::parse("checkout-r14-missing").unwrap(),
            )
            .await
            .unwrap_err(),
        StoreError::Invalid,
    );
    reopened.close().await;
    let _ = std::fs::remove_dir_all(fixture.root);
}

async fn pool(database: &std::path::Path) -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(database))
        .await
        .unwrap()
}
