use super::fixture::*;
use saya_store::{SqliteStateStore, StoreError};
use saya_types::{ApprovalDisposition, CodingEventKind};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::num::NonZeroU64;

#[tokio::test]
async fn expired_unclosed_requests_still_fill_pending_cap_and_pages_are_keyset_bounded() {
    let (path, store, job) = running("pending-cap").await;
    let expired = now().saturating_sub(100);
    for sequence in 2..130 {
        let req = request(
            &job,
            sequence,
            None,
            RequestSpec {
                digest: "a",
                epoch: 7,
                requested: expired.saturating_sub(1),
                expires: expired,
                host: "example.com",
            },
        );
        store
            .record_coding_approval_request(request_event(&req))
            .await
            .unwrap();
    }
    let excess = request(
        &job,
        130,
        None,
        RequestSpec {
            digest: "a",
            epoch: 7,
            requested: expired.saturating_sub(1),
            expires: expired,
            host: "example.com",
        },
    );
    assert_eq!(
        store
            .record_coding_approval_request(request_event(&excess))
            .await
            .unwrap_err(),
        StoreError::LimitExceeded
    );
    let first = store
        .list_coding_approval_requests(&job, None, 100)
        .await
        .unwrap();
    assert_eq!(first.items.len(), 100);
    assert_eq!(first.next_cursor, Some(NonZeroU64::new(101).unwrap()));
    let second = store
        .list_coding_approval_requests(&job, first.next_cursor, 100)
        .await
        .unwrap();
    assert_eq!(second.items.len(), 28);
    assert_eq!(
        second.items.first().unwrap().request.request_sequence(),
        NonZeroU64::new(102).unwrap()
    );
    assert_eq!(second.next_cursor, None);
    assert!(
        store
            .list_coding_approval_requests(&job, NonZeroU64::new(129), 100)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(
        store
            .list_coding_approval_requests(&job, None, 0)
            .await
            .unwrap_err(),
        StoreError::Invalid
    );
    assert_eq!(
        store
            .list_coding_approval_requests(&job, None, 101)
            .await
            .unwrap_err(),
        StoreError::LimitExceeded
    );
    store.close().await;
    let _ = std::fs::remove_dir_all(path);
}

#[tokio::test]
async fn lifetime_cap_accepts_ten_thousand_closed_requests_then_refuses_first_excess() {
    let (path, store, job) = running("lifetime-cap").await;
    let expired = now().saturating_sub(100);
    for index in 0..10_000_u64 {
        let request_sequence = index * 2 + 2;
        let req = request(
            &job,
            request_sequence,
            None,
            RequestSpec {
                digest: "a",
                epoch: 7,
                requested: expired.saturating_sub(1),
                expires: expired,
                host: "example.com",
            },
        );
        store
            .record_coding_approval_request(request_event(&req))
            .await
            .unwrap();
        let negative = event(
            request_sequence + 1,
            &job,
            None,
            CodingEventKind::ApprovalClosed {
                request_sequence: req.request_sequence(),
                disposition: ApprovalDisposition::Denied,
            },
        );
        store.close_coding_approval(negative).await.unwrap();
    }
    let excess_sequence = 20_002;
    let excess = request(
        &job,
        excess_sequence,
        None,
        RequestSpec {
            digest: "a",
            epoch: 7,
            requested: expired.saturating_sub(1),
            expires: expired,
            host: "example.com",
        },
    );
    assert_eq!(
        store
            .record_coding_approval_request(request_event(&excess))
            .await
            .unwrap_err(),
        StoreError::LimitExceeded
    );
    let page = store
        .list_coding_approval_requests(&job, None, 100)
        .await
        .unwrap();
    assert_eq!(page.items.len(), 100);
    assert_eq!(
        page.items[0].request.request_sequence(),
        NonZeroU64::new(2).unwrap()
    );
    store.close().await;
    let _ = std::fs::remove_dir_all(path);
}

#[tokio::test]
async fn reservation_storage_cap_rolls_back_consumption_and_keeps_request_pending() {
    let (path, store, job) = running("budget-cap").await;
    for sequence in 2..=1_025 {
        store
            .reserve_coding_budget(reservation(&job, sequence, None, 1))
            .await
            .unwrap();
    }
    let req = append_request(
        &store,
        &job,
        1_026,
        None,
        now().saturating_sub(1),
        now() + 60_000,
    )
    .await;
    assert_eq!(
        store
            .consume_coding_approval(
                reservation(&job, 1_027, None, 1),
                closure(&job, 1_026, 1_027, None),
                context(&req)
            )
            .await
            .unwrap_err(),
        StoreError::LimitExceeded
    );
    assert!(
        store
            .get_coding_approval_request(&job, req.request_sequence())
            .await
            .unwrap()
            .unwrap()
            .closure
            .is_none()
    );
    assert_eq!(
        store
            .list_coding_budget_reservations(&job, None, 100)
            .await
            .unwrap()
            .items
            .len(),
        100
    );
    store.close().await;
    let raw = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(path.join("state.sqlite3")))
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM coding_events WHERE job_id=?")
        .bind(job.as_str())
        .fetch_one(&raw)
        .await
        .unwrap();
    assert_eq!(count, 1_026);
    raw.close().await;
    let reopened = SqliteStateStore::new(path.join("state.sqlite3"));
    assert!(
        reopened
            .get_coding_approval_request(&job, req.request_sequence())
            .await
            .unwrap()
            .unwrap()
            .closure
            .is_none()
    );
    reopened.close().await;
    let _ = std::fs::remove_dir_all(path);
}
