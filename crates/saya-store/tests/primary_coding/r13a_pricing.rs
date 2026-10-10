use super::{
    r01::{create_job, temp_root},
    r02::event,
};
use saya_store::{SqliteStateStore, StoreError};
use saya_types::{
    BudgetCharge, BudgetDimension, BudgetEffectState, BudgetObservation, BudgetObservationValue,
    BudgetReservationV1, BudgetSettlementV1, CodingEventKind, JobState, Sha256Digest,
};
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

fn digest(letter: char) -> Sha256Digest {
    Sha256Digest::parse(&letter.to_string().repeat(64)).unwrap()
}

fn reservation(job: &saya_types::JobId) -> BudgetReservationV1 {
    BudgetReservationV1::new(
        job.clone(),
        NonZeroU64::new(2).unwrap(),
        None,
        None,
        vec![
            BudgetCharge::new(
                BudgetDimension::CostMicrousd,
                Some("provider-a".into()),
                Some(digest('a')),
                NonZeroU64::new(1).unwrap(),
            )
            .unwrap(),
        ],
    )
    .unwrap()
}

fn settlement(job: &saya_types::JobId, endpoint: &str, price: Sha256Digest) -> BudgetSettlementV1 {
    BudgetSettlementV1::new(
        job.clone(),
        NonZeroU64::new(2).unwrap(),
        NonZeroU64::new(3).unwrap(),
        BudgetEffectState::Settled,
        vec![
            BudgetObservation::new(
                BudgetDimension::CostMicrousd,
                Some(endpoint.into()),
                Some(price),
                BudgetObservationValue::Measured(1),
            )
            .unwrap(),
        ],
    )
    .unwrap()
}

async fn reserve(store: &SqliteStateStore, job: &saya_types::JobId) {
    store
        .reserve_coding_budget(event(
            2,
            job,
            None,
            CodingEventKind::BudgetReserved {
                reservation: Box::new(reservation(job)),
            },
        ))
        .await
        .unwrap();
}

#[tokio::test]
async fn settlement_refuses_price_and_endpoint_mismatches() {
    let (root, store, job) = running("r13a-price").await;
    reserve(&store, &job.job_id).await;
    for mismatch in [
        settlement(&job.job_id, "provider-a", digest('b')),
        settlement(&job.job_id, "provider-b", digest('a')),
    ] {
        assert_eq!(
            store
                .settle_coding_budget(event(
                    3,
                    &job.job_id,
                    None,
                    CodingEventKind::BudgetSettled {
                        settlement: Box::new(mismatch)
                    },
                ))
                .await
                .unwrap_err(),
            StoreError::Conflict
        );
    }
    assert!(
        store
            .get_coding_budget_reservation(&job.job_id, NonZeroU64::new(2).unwrap())
            .await
            .unwrap()
            .unwrap()
            .settlement
            .is_none()
    );
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn reads_refuse_tampered_settlement_price_metadata() {
    let (root, store, job) = running("r13a-read-price").await;
    let db = root.join("state.sqlite3");
    reserve(&store, &job.job_id).await;
    store
        .settle_coding_budget(event(
            3,
            &job.job_id,
            None,
            CodingEventKind::BudgetSettled {
                settlement: Box::new(settlement(&job.job_id, "provider-a", digest('a'))),
            },
        ))
        .await
        .unwrap();
    store.close().await;
    let raw = pool(&db).await;
    let tampered = event(
        3,
        &job.job_id,
        None,
        CodingEventKind::BudgetSettled {
            settlement: Box::new(settlement(&job.job_id, "provider-a", digest('b'))),
        },
    );
    sqlx::query("UPDATE coding_events SET event_json=? WHERE job_id=? AND sequence=3")
        .bind(String::from_utf8(tampered.to_json_bytes().unwrap()).unwrap())
        .bind(job.job_id.as_str())
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;
    let reopened = SqliteStateStore::new(&db);
    assert_eq!(
        reopened
            .get_coding_budget_reservation(&job.job_id, NonZeroU64::new(2).unwrap())
            .await
            .unwrap_err(),
        StoreError::Invalid
    );
    assert_eq!(
        reopened
            .list_coding_budget_reservations(&job.job_id, None, 1)
            .await
            .unwrap_err(),
        StoreError::Invalid
    );
    reopened.close().await;
    let _ = fs::remove_dir_all(root);
}
