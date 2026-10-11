use super::*;
use fixture::{CodingSeed, assert_coding_seed_survives, create_completed_checkout};
use saya_types::{ManagedCheckoutId, ManagedCheckoutLifecycle};
use std::num::NonZeroU64;

#[path = "checkout_upgrade/fixture.rs"]
mod fixture;

pub(super) async fn drop_step13_tables(pool: &SqlitePool) {
    drop_step14_table(pool).await;
    sqlx::query("DROP TABLE coding_checkout_operations")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE coding_managed_checkouts")
        .execute(pool)
        .await
        .unwrap();
}

pub(super) async fn drop_step14_table(pool: &SqlitePool) {
    sqlx::query("DROP TABLE coding_approval_requests")
        .execute(pool)
        .await
        .unwrap();
}

pub(super) async fn drop_step15_table(pool: &SqlitePool) {
    sqlx::query("DROP TABLE coding_budget_policies")
        .execute(pool)
        .await
        .unwrap();
}

pub(super) async fn drop_step16_table(pool: &SqlitePool) {
    sqlx::query("DROP TABLE IF EXISTS coding_cancel_requests")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE IF EXISTS coding_runtime_authority")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE IF EXISTS coding_completions")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE IF EXISTS coding_checkout_plans")
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn upgrade_from_twelve_preserves_jobs_children_events_and_budget_records() {
    let seed = CodingSeed::new("upgrade-v12-checkouts").await;
    let db = seed.database.clone();
    seed.store.close().await;

    let raw = create_pool(&db).await;
    drop_step16_table(&raw).await;
    drop_step13_tables(&raw).await;
    drop_step15_table(&raw).await;
    sqlx::query("PRAGMA user_version = 12")
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;
    assert_eq!(user_version(&db).await, 12);

    let upgraded = SqliteStateStore::new(&db);
    assert_coding_seed_survives(&upgraded, &seed, &seed.events).await;
    upgraded.close().await;
    assert_eq!(user_version(&db).await, 17);
    assert!(table_exists(&db, "coding_managed_checkouts").await);
    assert!(table_exists(&db, "coding_checkout_operations").await);
    assert!(table_exists(&db, "coding_approval_requests").await);
    let _ = fs::remove_dir_all(seed.root);
}

#[tokio::test]
async fn upgrade_from_thirteen_preserves_completed_checkout_ownership_and_operation() {
    let mut seed = CodingSeed::new("upgrade-v13-checkout-data").await;
    let (ownership, completed_operation, create_event, outcome_event) =
        create_completed_checkout(&mut seed).await;
    let mut expected_events = seed.events.clone();
    expected_events.extend([create_event, outcome_event]);
    let db = seed.database.clone();
    seed.store.close().await;

    let raw = create_pool(&db).await;
    drop_step16_table(&raw).await;
    drop_step14_table(&raw).await;
    drop_step15_table(&raw).await;
    sqlx::query("PRAGMA user_version = 13")
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;
    assert_eq!(user_version(&db).await, 13);

    let upgraded = SqliteStateStore::new(&db);
    assert_coding_seed_survives(&upgraded, &seed, &expected_events).await;
    let checkout_id = ManagedCheckoutId::parse("checkout-v13-preserved").unwrap();
    let checkout = upgraded
        .get_managed_checkout(&seed.job_id, &checkout_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(checkout.ownership, ownership);
    assert_eq!(
        checkout.ownership.managed_path().as_str(),
        "/tmp/saya-r13b-preserved-checkout"
    );
    assert_eq!(checkout.current_lifecycle, ManagedCheckoutLifecycle::Active);
    assert_eq!(checkout.latest_operation, completed_operation);
    assert_eq!(
        upgraded
            .get_checkout_operation(&seed.job_id, &checkout_id, NonZeroU64::new(4).unwrap(),)
            .await
            .unwrap(),
        Some(completed_operation.clone())
    );
    assert_eq!(
        upgraded
            .list_checkout_operations(&seed.job_id, &checkout_id, None, 10)
            .await
            .unwrap()
            .items,
        vec![completed_operation]
    );
    upgraded.close().await;
    assert_eq!(user_version(&db).await, 17);
    let _ = fs::remove_dir_all(seed.root);
}

#[tokio::test]
async fn upgrade_from_sixteen_preserves_existing_records_and_adds_result_delivery() {
    let seed = CodingSeed::new("upgrade-v16-results").await;
    let db = seed.database.clone();
    seed.store.close().await;
    let raw = create_pool(&db).await;
    sqlx::query("DROP TABLE coding_cancel_requests")
        .execute(&raw)
        .await
        .unwrap();
    sqlx::query("DROP TABLE coding_runtime_authority")
        .execute(&raw)
        .await
        .unwrap();
    sqlx::query("DROP TABLE coding_completions")
        .execute(&raw)
        .await
        .unwrap();
    sqlx::query("PRAGMA user_version = 16")
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;
    let upgraded = SqliteStateStore::new(&db);
    assert_coding_seed_survives(&upgraded, &seed, &seed.events).await;
    assert!(
        upgraded
            .list_coding_completions(&seed.job_id, None, 32)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    upgraded.close().await;
    assert_eq!(user_version(&db).await, 17);
    assert!(table_exists(&db, "coding_completions").await);
    fs::remove_dir_all(seed.root).unwrap();
}
