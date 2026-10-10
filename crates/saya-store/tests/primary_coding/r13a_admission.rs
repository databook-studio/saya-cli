use super::{
    r01::{child, create_job, temp_root},
    r02::event,
    r13a::reservation,
};
use saya_store::{CodingEventAppend, SqliteStateStore, StoreError};
use saya_types::{
    AgentId, BudgetReservationV1, CodingEventKind, CodingEventV1, InvocationId, JobState,
};
use std::{fs, num::NonZeroU64, sync::Arc};

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

#[tokio::test]
async fn stale_partial_unknown_and_denied_reservations_are_refused() {
    let (root, store, job) = running("r13a-actors").await;
    let child = store
        .create_coding_child(child(job.job_id.clone(), "r13a-actors"))
        .await
        .unwrap();
    let stale_agent = child.agent_id.clone();
    let stale_invocation = InvocationId::parse("invocation-stale").unwrap();
    let stale_reservation = BudgetReservationV1::new(
        job.job_id.clone(),
        NonZeroU64::new(2).unwrap(),
        Some(stale_agent.clone()),
        Some(stale_invocation.clone()),
        vec![super::r13a::charge(1)],
    )
    .unwrap();
    let stale = CodingEventV1::new(
        NonZeroU64::new(2).unwrap(),
        job.job_id.clone(),
        Some(stale_agent),
        Some(stale_invocation),
        CodingEventKind::BudgetReserved {
            reservation: Box::new(stale_reservation),
        },
    )
    .unwrap();
    assert_eq!(
        store.reserve_coding_budget(stale).await.unwrap_err(),
        StoreError::Conflict
    );

    let unknown_agent = AgentId::parse("agent-unknown").unwrap();
    let unknown_invocation = InvocationId::parse("invocation-unknown").unwrap();
    let unknown_reservation = BudgetReservationV1::new(
        job.job_id.clone(),
        NonZeroU64::new(2).unwrap(),
        Some(unknown_agent.clone()),
        Some(unknown_invocation.clone()),
        vec![super::r13a::charge(1)],
    )
    .unwrap();
    let unknown = CodingEventV1::new(
        NonZeroU64::new(2).unwrap(),
        job.job_id.clone(),
        Some(unknown_agent),
        Some(unknown_invocation),
        CodingEventKind::BudgetReserved {
            reservation: Box::new(unknown_reservation),
        },
    )
    .unwrap();
    assert_eq!(
        store.reserve_coding_budget(unknown).await.unwrap_err(),
        StoreError::NotFound
    );

    let partial_agent = AgentId::parse("agent-partial").unwrap();
    let partial_reservation = BudgetReservationV1::new(
        job.job_id.clone(),
        NonZeroU64::new(2).unwrap(),
        Some(partial_agent.clone()),
        None,
        vec![super::r13a::charge(1)],
    )
    .unwrap();
    let partial = CodingEventV1::new(
        NonZeroU64::new(2).unwrap(),
        job.job_id.clone(),
        Some(partial_agent),
        None,
        CodingEventKind::BudgetReserved {
            reservation: Box::new(partial_reservation),
        },
    )
    .unwrap();
    assert_eq!(
        store.reserve_coding_budget(partial).await.unwrap_err(),
        StoreError::Invalid
    );
    store
        .append_coding_event(event(
            2,
            &job.job_id,
            None,
            CodingEventKind::StateChanged {
                state: JobState::WaitingForApproval,
            },
        ))
        .await
        .unwrap();
    assert_eq!(
        store
            .reserve_coding_budget(event(
                3,
                &job.job_id,
                None,
                CodingEventKind::BudgetReserved {
                    reservation: Box::new(reservation(&job.job_id, 3, None)),
                },
            ))
            .await
            .unwrap_err(),
        StoreError::Conflict
    );
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn queued_and_paused_jobs_refuse_new_root_reservations() {
    let root = temp_root("r13a-denied-states");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let job = create_job(&store, "r13a-denied-states").await;
    assert_eq!(
        store
            .reserve_coding_budget(event(
                1,
                &job.job_id,
                None,
                CodingEventKind::BudgetReserved {
                    reservation: Box::new(reservation(&job.job_id, 1, None)),
                },
            ))
            .await
            .unwrap_err(),
        StoreError::Conflict
    );
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
    store
        .append_coding_event(event(
            2,
            &job.job_id,
            None,
            CodingEventKind::StateChanged {
                state: JobState::Paused,
            },
        ))
        .await
        .unwrap();
    assert_eq!(
        store
            .reserve_coding_budget(event(
                3,
                &job.job_id,
                None,
                CodingEventKind::BudgetReserved {
                    reservation: Box::new(reservation(&job.job_id, 3, None)),
                },
            ))
            .await
            .unwrap_err(),
        StoreError::Conflict
    );
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn same_next_budget_sequence_has_exactly_one_winner() {
    let root = temp_root("r13a-race");
    let db = root.join("state.sqlite3");
    let setup = SqliteStateStore::new(&db);
    let job = create_job(&setup, "r13a-race").await;
    setup
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
    setup.close().await;
    let first = Arc::new(SqliteStateStore::new(&db));
    let second = Arc::new(SqliteStateStore::new(&db));
    let one = event(
        2,
        &job.job_id,
        None,
        CodingEventKind::BudgetReserved {
            reservation: Box::new(reservation(&job.job_id, 2, None)),
        },
    );
    let two = CodingEventV1::new(
        NonZeroU64::new(2).unwrap(),
        job.job_id.clone(),
        None,
        None,
        CodingEventKind::BudgetReserved {
            reservation: Box::new(
                BudgetReservationV1::new(
                    job.job_id.clone(),
                    NonZeroU64::new(2).unwrap(),
                    None,
                    None,
                    vec![super::r13a::charge(2)],
                )
                .unwrap(),
            ),
        },
    )
    .unwrap();
    let (left, right) = tokio::join!(
        first.reserve_coding_budget(one),
        second.reserve_coding_budget(two)
    );
    let results = [left, right];
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Ok(CodingEventAppend::Appended(_))))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| **result == Err(StoreError::Conflict))
            .count(),
        1
    );
    first.close().await;
    second.close().await;
    let _ = fs::remove_dir_all(root);
}
