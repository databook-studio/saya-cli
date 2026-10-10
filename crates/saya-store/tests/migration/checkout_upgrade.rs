use super::*;
use saya_store::{NewCodingChild, NewCodingJob};
use saya_types::{
    AgentId, BudgetCharge, BudgetDimension, BudgetEffectState, BudgetObservation,
    BudgetObservationValue, BudgetReservationV1, BudgetSettlementV1, CodingEventKind,
    CodingEventV1, InvocationId, JobId, JobState, RepositoryId, SnapshotId,
};
use std::num::NonZeroU64;

pub(super) async fn drop_step13_tables(pool: &SqlitePool) {
    sqlx::query("DROP TABLE coding_checkout_operations")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DROP TABLE coding_managed_checkouts")
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn upgrade_from_twelve_preserves_jobs_children_events_and_budget_records() {
    let root = temp_root("upgrade-v12-checkouts");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let job_id = JobId::parse("job-v12-checkout").unwrap();
    store
        .create_coding_job(NewCodingJob {
            job_id: job_id.clone(),
            repository_id: RepositoryId::parse("repository-v12-checkout").unwrap(),
            snapshot_id: SnapshotId::parse("snapshot-v12-checkout").unwrap(),
        })
        .await
        .unwrap();
    let child = store
        .create_coding_child(NewCodingChild {
            job_id: job_id.clone(),
            agent_id: AgentId::parse("agent-v12-checkout").unwrap(),
            current_invocation_id: InvocationId::parse("invocation-v12-checkout").unwrap(),
        })
        .await
        .unwrap();
    let running = CodingEventV1::new(
        NonZeroU64::new(1).unwrap(),
        job_id.clone(),
        None,
        None,
        CodingEventKind::StateChanged {
            state: JobState::Running,
        },
    )
    .unwrap();
    store.append_coding_event(running.clone()).await.unwrap();
    let reservation = BudgetReservationV1::new(
        job_id.clone(),
        NonZeroU64::new(2).unwrap(),
        Some(child.agent_id.clone()),
        Some(child.current_invocation_id.clone()),
        vec![
            BudgetCharge::new(
                BudgetDimension::ChildSlots,
                None,
                None,
                NonZeroU64::new(1).unwrap(),
            )
            .unwrap(),
        ],
    )
    .unwrap();
    let reserved = CodingEventV1::new(
        NonZeroU64::new(2).unwrap(),
        job_id.clone(),
        Some(child.agent_id.clone()),
        Some(child.current_invocation_id.clone()),
        CodingEventKind::BudgetReserved {
            reservation: Box::new(reservation.clone()),
        },
    )
    .unwrap();
    store.reserve_coding_budget(reserved.clone()).await.unwrap();
    let settlement = BudgetSettlementV1::new(
        job_id.clone(),
        NonZeroU64::new(2).unwrap(),
        NonZeroU64::new(3).unwrap(),
        BudgetEffectState::Settled,
        vec![
            BudgetObservation::new(
                BudgetDimension::ChildSlots,
                None,
                None,
                BudgetObservationValue::Measured(1),
            )
            .unwrap(),
        ],
    )
    .unwrap();
    let settled = CodingEventV1::new(
        NonZeroU64::new(3).unwrap(),
        job_id.clone(),
        None,
        None,
        CodingEventKind::BudgetSettled {
            settlement: Box::new(settlement.clone()),
        },
    )
    .unwrap();
    store.settle_coding_budget(settled.clone()).await.unwrap();
    store.close().await;

    let raw = create_pool(&db).await;
    drop_step13_tables(&raw).await;
    sqlx::query("PRAGMA user_version = 12")
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;

    let upgraded = SqliteStateStore::new(&db);
    assert_eq!(
        upgraded
            .get_coding_job(&job_id)
            .await
            .unwrap()
            .unwrap()
            .state,
        JobState::Running,
    );
    assert_eq!(
        upgraded
            .list_coding_children(&job_id, None, 1)
            .await
            .unwrap()
            .items,
        vec![child],
    );
    assert_eq!(
        upgraded
            .list_coding_events(&job_id, None, 10)
            .await
            .unwrap()
            .events,
        vec![running, reserved, settled],
    );
    let budget = upgraded
        .get_coding_budget_reservation(&job_id, NonZeroU64::new(2).unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(budget.reservation, reservation);
    assert_eq!(budget.settlement, Some(settlement));
    upgraded.close().await;
    assert_eq!(user_version(&db).await, 13);
    assert!(table_exists(&db, "coding_managed_checkouts").await);
    assert!(table_exists(&db, "coding_checkout_operations").await);
    let _ = fs::remove_dir_all(root);
}
