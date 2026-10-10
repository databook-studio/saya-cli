use super::{
    r01::{create_job, temp_root},
    r02::event,
    r13a::{reservation, settlement},
};
use saya_store::{CodingEventAppend, SqliteStateStore, StoreError};
use saya_types::{CodingEventKind, JobState};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{fs, num::NonZeroU64, path::Path};

async fn pool(path: &Path) -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(path))
        .await
        .unwrap()
}

async fn running(
    label: &str,
) -> (
    std::path::PathBuf,
    SqliteStateStore,
    saya_store::CodingJobRecord,
) {
    let root = temp_root(label);
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let job = create_job(&store, label).await;
    store
        .append_coding_event(event(
            1,
            &job.job_id,
            None,
            CodingEventKind::StateChanged {
                state: JobState::Running,
            },
        ))
        .await
        .unwrap();
    (root, store, job)
}

async fn seed_unsettled(path: &Path, job: &saya_types::JobId, count: u64) {
    let pool = pool(path).await;
    let mut transaction = pool.begin().await.unwrap();
    for offset in 0..count {
        let sequence = 2 + offset;
        let reserved = event(
            sequence,
            job,
            None,
            CodingEventKind::BudgetReserved {
                reservation: Box::new(reservation(job, sequence, None)),
            },
        );
        sqlx::query("INSERT INTO coding_events(job_id, sequence, event_json) VALUES (?, ?, ?)")
            .bind(job.as_str())
            .bind(sequence as i64)
            .bind(String::from_utf8(reserved.to_json_bytes().unwrap()).unwrap())
            .execute(&mut *transaction)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO coding_budget_reservations(job_id, reservation_sequence) VALUES (?, ?)",
        )
        .bind(job.as_str())
        .bind(sequence as i64)
        .execute(&mut *transaction)
        .await
        .unwrap();
    }
    transaction.commit().await.unwrap();
    pool.close().await;
}

async fn seed_settled(path: &Path, job: &saya_types::JobId, count: u64) {
    let pool = pool(path).await;
    let mut transaction = pool.begin().await.unwrap();
    for offset in 0..count {
        let reservation_sequence = 2 + offset * 2;
        let settlement_sequence = reservation_sequence + 1;
        let reserved = event(
            reservation_sequence,
            job,
            None,
            CodingEventKind::BudgetReserved {
                reservation: Box::new(reservation(job, reservation_sequence, None)),
            },
        );
        let settled = event(
            settlement_sequence,
            job,
            None,
            CodingEventKind::BudgetSettled {
                settlement: Box::new(settlement(job, reservation_sequence, settlement_sequence)),
            },
        );
        for event in [reserved, settled] {
            sqlx::query("INSERT INTO coding_events(job_id, sequence, event_json) VALUES (?, ?, ?)")
                .bind(job.as_str())
                .bind(event.sequence().get() as i64)
                .bind(String::from_utf8(event.to_json_bytes().unwrap()).unwrap())
                .execute(&mut *transaction)
                .await
                .unwrap();
        }
        sqlx::query("INSERT INTO coding_budget_reservations(job_id, reservation_sequence, settled_by_sequence) VALUES (?, ?, ?)")
            .bind(job.as_str())
            .bind(reservation_sequence as i64)
            .bind(settlement_sequence as i64)
            .execute(&mut *transaction)
            .await
            .unwrap();
    }
    transaction.commit().await.unwrap();
    pool.close().await;
}

#[tokio::test]
async fn exact_unsettled_and_lifetime_bounds_refuse_one_more_reservation() {
    let (root, store, job) = running("r13a-unsettled-cap").await;
    let db = root.join("state.sqlite3");
    store.close().await;
    seed_unsettled(&db, &job.job_id, 1_023).await;
    let store = SqliteStateStore::new(&db);
    let last_unsettled = event(
        1_025,
        &job.job_id,
        None,
        CodingEventKind::BudgetReserved {
            reservation: Box::new(reservation(&job.job_id, 1_025, None)),
        },
    );
    assert_eq!(
        store.reserve_coding_budget(last_unsettled).await.unwrap(),
        CodingEventAppend::Appended(NonZeroU64::new(1_025).unwrap())
    );
    assert!(
        store
            .get_coding_budget_reservation(&job.job_id, NonZeroU64::new(1_025).unwrap())
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        store
            .reserve_coding_budget(event(
                1_026,
                &job.job_id,
                None,
                CodingEventKind::BudgetReserved {
                    reservation: Box::new(reservation(&job.job_id, 1_026, None)),
                },
            ))
            .await
            .unwrap_err(),
        StoreError::LimitExceeded
    );
    store.close().await;
    let _ = fs::remove_dir_all(root);

    let (root, store, job) = running("r13a-total-cap").await;
    let db = root.join("state.sqlite3");
    store.close().await;
    seed_settled(&db, &job.job_id, 9_999).await;
    let store = SqliteStateStore::new(&db);
    let last_total = event(
        20_000,
        &job.job_id,
        None,
        CodingEventKind::BudgetReserved {
            reservation: Box::new(reservation(&job.job_id, 20_000, None)),
        },
    );
    assert_eq!(
        store.reserve_coding_budget(last_total).await.unwrap(),
        CodingEventAppend::Appended(NonZeroU64::new(20_000).unwrap())
    );
    assert_eq!(
        store
            .list_coding_budget_reservations(
                &job.job_id,
                Some(NonZeroU64::new(19_998).unwrap()),
                1,
            )
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    assert_eq!(
        store
            .reserve_coding_budget(event(
                20_001,
                &job.job_id,
                None,
                CodingEventKind::BudgetReserved {
                    reservation: Box::new(reservation(&job.job_id, 20_001, None)),
                },
            ))
            .await
            .unwrap_err(),
        StoreError::LimitExceeded
    );
    store.close().await;
    let _ = fs::remove_dir_all(root);
}
