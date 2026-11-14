use super::fixture::*;
use saya_store::{CodingApprovalConsumptionAppend, SqliteStateStore, StoreError};
use std::{num::NonZeroU64, sync::Arc};

async fn competing_consumers(
    label: &str,
    distinct: bool,
) -> [Result<CodingApprovalConsumptionAppend, StoreError>; 2] {
    let (path, setup, job) = running(label).await;
    let req = append_request(
        &setup,
        &job,
        2,
        None,
        now().saturating_sub(1),
        now() + 60_000,
    )
    .await;
    setup.close().await;
    let first = Arc::new(SqliteStateStore::new(path.join("state.sqlite3")));
    let second = Arc::new(SqliteStateStore::new(path.join("state.sqlite3")));
    let one = (
        reservation(&job, 3, None, 1),
        closure(&job, 2, 3, None),
        context(&req),
    );
    let two = if distinct {
        (
            reservation(&job, 3, None, 2),
            closure(&job, 2, 3, None),
            context(&req),
        )
    } else {
        one.clone()
    };
    let (left, right) = tokio::join!(
        first.consume_coding_approval(one.0, one.1, one.2),
        second.consume_coding_approval(two.0, two.1, two.2)
    );
    first.close().await;
    second.close().await;
    let check = SqliteStateStore::new(path.join("state.sqlite3"));
    let saved = check
        .get_coding_approval_request(&job, NonZeroU64::new(2).unwrap())
        .await
        .unwrap()
        .unwrap();
    assert!(saved.closure.is_some());
    assert_eq!(
        check
            .list_coding_budget_reservations(&job, None, 10)
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    check.close().await;
    let _ = std::fs::remove_dir_all(path);
    [left, right]
}

#[tokio::test]
async fn concurrent_identical_pair_has_one_record_and_one_audit_duplicate() {
    let results = competing_consumers("concurrent-identical", false).await;
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Ok(CodingApprovalConsumptionAppend::Recorded(_))))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Ok(CodingApprovalConsumptionAppend::AlreadyPresent(_))))
            .count(),
        1
    );
}

#[tokio::test]
async fn concurrent_distinct_pairs_have_one_record_and_one_conflict() {
    let results = competing_consumers("concurrent-distinct", true).await;
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Ok(CodingApprovalConsumptionAppend::Recorded(_))))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|r| **r == Err(StoreError::Conflict))
            .count(),
        1
    );
}
