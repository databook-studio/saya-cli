use super::fixture::*;
use saya_store::SqliteStateStore;
use saya_types::{CodingEventKind, JobState};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

async fn consumed_db(label: &str) -> (std::path::PathBuf, saya_types::JobId) {
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
    store
        .consume_coding_approval(
            reservation(&job, 3, None, 1),
            closure(&job, 2, 3, None),
            context(&req),
        )
        .await
        .unwrap();
    store.close().await;
    (path, job)
}

async fn raw_pool(path: &std::path::Path) -> sqlx::SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(path.join("state.sqlite3"))
                .foreign_keys(false),
        )
        .await
        .unwrap()
}

async fn both_reads_fail(path: &std::path::Path, job: &saya_types::JobId) {
    let store = SqliteStateStore::new(path.join("state.sqlite3"));
    assert!(
        store
            .get_coding_approval_request(job, std::num::NonZeroU64::new(2).unwrap())
            .await
            .is_err()
    );
    assert!(
        store
            .list_coding_approval_requests(job, None, 10)
            .await
            .is_err()
    );
    store.close().await;
}

#[tokio::test]
async fn consumed_read_rejects_missing_projection_and_valid_typed_foreign_actor() {
    let (path, job) = consumed_db("missing-reservation-projection").await;
    let raw = raw_pool(&path).await;
    sqlx::query("DELETE FROM coding_budget_reservations WHERE job_id=? AND reservation_sequence=3")
        .bind(job.as_str())
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;
    both_reads_fail(&path, &job).await;
    let _ = std::fs::remove_dir_all(&path);

    let (path, store, job) = running("foreign-reservation-actor").await;
    let actor = child(&store, &job, "foreign-reservation").await;
    let req = append_request(
        &store,
        &job,
        2,
        None,
        now().saturating_sub(1),
        now() + 60_000,
    )
    .await;
    store
        .consume_coding_approval(
            reservation(&job, 3, None, 1),
            closure(&job, 2, 3, None),
            context(&req),
        )
        .await
        .unwrap();
    store.close().await;
    let foreign = reservation(&job, 3, Some((&actor.0, &actor.1)), 1);
    let json = String::from_utf8(foreign.to_json_bytes().unwrap()).unwrap();
    let raw = raw_pool(&path).await;
    sqlx::query("UPDATE coding_events SET event_json=? WHERE job_id=? AND sequence=3")
        .bind(json)
        .bind(job.as_str())
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;
    both_reads_fail(&path, &job).await;
    let _ = std::fs::remove_dir_all(path);
}

#[tokio::test]
async fn missing_request_or_closure_event_and_future_consumed_reference_fail_closed() {
    for (label, sequence) in [
        ("missing-request-event", 2_i64),
        ("missing-closure-event", 4_i64),
    ] {
        let (path, job) = consumed_db(label).await;
        let raw = raw_pool(&path).await;
        sqlx::query("DELETE FROM coding_events WHERE job_id=? AND sequence=?")
            .bind(job.as_str())
            .bind(sequence)
            .execute(&raw)
            .await
            .unwrap();
        raw.close().await;
        both_reads_fail(&path, &job).await;
        let _ = std::fs::remove_dir_all(path);
    }

    let (path, job) = consumed_db("missing-reservation-event").await;
    let raw = raw_pool(&path).await;
    sqlx::query("DELETE FROM coding_events WHERE job_id=? AND sequence=3")
        .bind(job.as_str())
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;
    both_reads_fail(&path, &job).await;
    let _ = std::fs::remove_dir_all(path);

    let (path, job) = consumed_db("future-reservation-reference").await;
    let raw = raw_pool(&path).await;
    let json: String =
        sqlx::query_scalar("SELECT event_json FROM coding_events WHERE job_id=? AND sequence=4")
            .bind(job.as_str())
            .fetch_one(&raw)
            .await
            .unwrap();
    let future = json.replace("\"reservation_sequence\":3", "\"reservation_sequence\":99");
    assert_ne!(future, json);
    sqlx::query("UPDATE coding_events SET event_json=? WHERE job_id=? AND sequence=4")
        .bind(future)
        .bind(job.as_str())
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;
    both_reads_fail(&path, &job).await;
    let _ = std::fs::remove_dir_all(path);
}

#[tokio::test]
async fn future_or_wrong_kind_request_event_fails_both_read_apis() {
    let (path, store, job) = running("future-request-event").await;
    append_request(
        &store,
        &job,
        2,
        None,
        now().saturating_sub(1),
        now() + 60_000,
    )
    .await;
    store.close().await;
    let raw = raw_pool(&path).await;
    let json: String =
        sqlx::query_scalar("SELECT event_json FROM coding_events WHERE job_id=? AND sequence=2")
            .bind(job.as_str())
            .fetch_one(&raw)
            .await
            .unwrap();
    let future = json.replace("\"version\":1", "\"version\":2");
    assert_ne!(future, json);
    sqlx::query("UPDATE coding_events SET event_json=? WHERE job_id=? AND sequence=2")
        .bind(future)
        .bind(job.as_str())
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;
    both_reads_fail(&path, &job).await;
    let _ = std::fs::remove_dir_all(&path);

    let (path, job) = consumed_db("wrong-kind-reservation-event").await;
    let replacement = event(
        3,
        &job,
        None,
        CodingEventKind::StateChanged {
            state: JobState::Running,
        },
    )
    .to_json_bytes()
    .unwrap();
    let raw = raw_pool(&path).await;
    sqlx::query("UPDATE coding_events SET event_json=? WHERE job_id=? AND sequence=3")
        .bind(replacement)
        .bind(job.as_str())
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;
    both_reads_fail(&path, &job).await;
    let _ = std::fs::remove_dir_all(path);

    let (path, store, job) = running("wrong-kind-request-event").await;
    append_request(
        &store,
        &job,
        2,
        None,
        now().saturating_sub(1),
        now() + 60_000,
    )
    .await;
    store.close().await;
    let replacement = event(
        2,
        &job,
        None,
        CodingEventKind::StateChanged {
            state: JobState::Running,
        },
    )
    .to_json_bytes()
    .unwrap();
    let raw = raw_pool(&path).await;
    sqlx::query("UPDATE coding_events SET event_json=? WHERE job_id=? AND sequence=2")
        .bind(replacement)
        .bind(job.as_str())
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;
    both_reads_fail(&path, &job).await;
    let _ = std::fs::remove_dir_all(path);
}
