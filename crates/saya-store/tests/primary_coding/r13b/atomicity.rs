use super::fixture::*;
use saya_store::{CodingEventAppend, SqliteStateStore, StoreError};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

#[tokio::test]
async fn trigger_failures_after_pair_append_roll_back_every_effect_and_retry() {
    for (label, trigger, drop) in [
        (
            "budget-trigger",
            "CREATE TRIGGER reject_r13b_budget BEFORE INSERT ON coding_budget_reservations BEGIN SELECT RAISE(ABORT, 'reject'); END",
            "DROP TRIGGER reject_r13b_budget",
        ),
        (
            "closure-trigger",
            "CREATE TRIGGER reject_r13b_closure BEFORE UPDATE OF closed_by_sequence ON coding_approval_requests BEGIN SELECT RAISE(ABORT, 'reject'); END",
            "DROP TRIGGER reject_r13b_closure",
        ),
    ] {
        let (path, store, job) = running(label).await;
        let req = append_request(
            &store,
            &job,
            2,
            None,
            now().saturating_sub(1),
            now() + 60_000,
        )
        .await;
        store.close().await;
        let db = path.join("state.sqlite3");
        let raw = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(SqliteConnectOptions::new().filename(&db))
            .await
            .unwrap();
        sqlx::query(trigger).execute(&raw).await.unwrap();
        raw.close().await;
        let blocked = SqliteStateStore::new(&db);
        let pair = (
            reservation(&job, 3, None, 1),
            closure(&job, 2, 3, None),
            context(&req),
        );
        assert_eq!(
            blocked
                .consume_coding_approval(pair.0.clone(), pair.1.clone(), pair.2.clone())
                .await
                .unwrap_err(),
            StoreError::Unavailable,
            "{label}"
        );
        assert!(
            blocked
                .get_coding_approval_request(&job, req.request_sequence())
                .await
                .unwrap()
                .unwrap()
                .closure
                .is_none()
        );
        blocked.close().await;
        let raw = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(SqliteConnectOptions::new().filename(&db))
            .await
            .unwrap();
        let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM coding_events WHERE job_id=?")
            .bind(job.as_str())
            .fetch_one(&raw)
            .await
            .unwrap();
        let budgets: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM coding_budget_reservations WHERE job_id=?")
                .bind(job.as_str())
                .fetch_one(&raw)
                .await
                .unwrap();
        let closure_sequence: Option<i64> = sqlx::query_scalar("SELECT closed_by_sequence FROM coding_approval_requests WHERE job_id=? AND request_sequence=2").bind(job.as_str()).fetch_one(&raw).await.unwrap();
        assert_eq!((events, budgets, closure_sequence), (2, 0, None), "{label}");
        sqlx::query(drop).execute(&raw).await.unwrap();
        raw.close().await;
        let retry = SqliteStateStore::new(&db);
        assert!(
            retry
                .consume_coding_approval(pair.0, pair.1, pair.2)
                .await
                .unwrap()
                .eq(&saya_store::CodingApprovalConsumptionAppend::Recorded(
                    saya_store::CodingApprovalConsumptionReceipt {
                        reservation_sequence: std::num::NonZeroU64::new(3).unwrap(),
                        closure_sequence: std::num::NonZeroU64::new(4).unwrap()
                    }
                ))
        );
        assert_eq!(
            retry
                .get_coding_approval_request(&job, req.request_sequence())
                .await
                .unwrap()
                .unwrap()
                .closure
                .unwrap()
                .closure_sequence
                .get(),
            4
        );
        retry.close().await;
        let _ = std::fs::remove_dir_all(path);
    }
}

#[tokio::test]
async fn an_existing_ordinary_reservation_cannot_be_consumed_retroactively() {
    let (path, store, job) = running("ordinary-reservation").await;
    let req = append_request(
        &store,
        &job,
        2,
        None,
        now().saturating_sub(1),
        now() + 60_000,
    )
    .await;
    let reservation = reservation(&job, 3, None, 1);
    assert_eq!(
        store
            .reserve_coding_budget(reservation.clone())
            .await
            .unwrap(),
        CodingEventAppend::Appended(reservation.sequence())
    );
    assert_eq!(
        store
            .consume_coding_approval(reservation, closure(&job, 2, 3, None), context(&req))
            .await
            .unwrap_err(),
        StoreError::Conflict
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
            .list_coding_events(&job, None, 10)
            .await
            .unwrap()
            .events
            .len(),
        3
    );
    store.close().await;
    let _ = std::fs::remove_dir_all(path);
}
