use super::{
    r01::{child, create_job, temp_root},
    r02::event,
};
use saya_store::{CodingRuntimeAuthority, SqliteStateStore, StoreError};
use saya_types::{
    BudgetCharge, BudgetDimension, BudgetReservationV1, CodingEventKind, JobState, Sha256Digest,
};
use std::num::NonZeroU64;

#[tokio::test]
async fn renewal_fences_stale_admission_and_refuses_unresolved_effects() {
    let root = temp_root("runtime-renewal");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let job = create_job(&store, "renewal").await;
    let child = store
        .create_coding_child(child(job.job_id.clone(), "renewal"))
        .await
        .unwrap();
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
    let first = CodingRuntimeAuthority {
        epoch: 1,
        policy_digest: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
    };
    store
        .bind_coding_runtime(&job.job_id, &first)
        .await
        .unwrap();
    let fresh = CodingRuntimeAuthority {
        epoch: 2,
        policy_digest: first.policy_digest.clone(),
    };
    store
        .renew_coding_runtime(&job.job_id, Some(&first), &fresh, 1)
        .await
        .unwrap();
    let reservation = BudgetReservationV1::new(
        job.job_id.clone(),
        NonZeroU64::new(3).unwrap(),
        Some(child.agent_id.clone()),
        Some(child.current_invocation_id.clone()),
        vec![
            BudgetCharge::new(
                BudgetDimension::ToolCalls,
                None,
                None,
                NonZeroU64::new(1).unwrap(),
            )
            .unwrap(),
        ],
    )
    .unwrap();
    let reserve = event(
        3,
        &job.job_id,
        Some(&child),
        CodingEventKind::BudgetReserved {
            reservation: Box::new(reservation),
        },
    );
    assert_eq!(
        store
            .reserve_coding_budget_fenced(reserve.clone(), Some(&first))
            .await
            .unwrap_err(),
        StoreError::Conflict
    );
    assert_eq!(
        store
            .reserve_coding_budget(reserve.clone())
            .await
            .unwrap_err(),
        StoreError::Conflict
    );
    store
        .reserve_coding_budget_fenced(reserve, Some(&fresh))
        .await
        .unwrap();
    let status = store.coding_recovery_status(&job.job_id).await.unwrap();
    assert_eq!(status.unresolved_reservations, 1);
    assert_eq!(status.last_sequence, 3);
    let next = CodingRuntimeAuthority {
        epoch: 3,
        ..fresh.clone()
    };
    assert_eq!(
        store
            .renew_coding_runtime(&job.job_id, Some(&fresh), &next, 3)
            .await
            .unwrap_err(),
        StoreError::Conflict
    );
    assert_eq!(
        store
            .coding_recovery_status(&job.job_id)
            .await
            .unwrap()
            .authority,
        Some(fresh)
    );
    assert_eq!(
        store
            .list_coding_budget_reservations(&job.job_id, None, 100)
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    store.close().await;
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn cancel_intent_survives_reopen_and_only_fresh_resume_clears_it() {
    let root = temp_root("cancel-intent");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let job = create_job(&store, "cancel-intent").await;
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
    assert!(store.request_coding_cancel(&job.job_id).await.unwrap());
    assert!(!store.request_coding_cancel(&job.job_id).await.unwrap());
    store.close().await;
    let store = SqliteStateStore::new(&db);
    assert!(store.coding_cancel_requested(&job.job_id).await.unwrap());
    assert_eq!(
        store
            .get_coding_job(&job.job_id)
            .await
            .unwrap()
            .unwrap()
            .state,
        JobState::Running
    );
    let fresh = CodingRuntimeAuthority {
        epoch: 1,
        policy_digest: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
    };
    store
        .renew_coding_runtime(&job.job_id, None, &fresh, 1)
        .await
        .unwrap();
    assert!(!store.coding_cancel_requested(&job.job_id).await.unwrap());
    assert!(store.request_coding_cancel(&job.job_id).await.unwrap());
    let reservation = BudgetReservationV1::new(
        job.job_id.clone(),
        NonZeroU64::new(3).unwrap(),
        None,
        None,
        vec![
            BudgetCharge::new(
                BudgetDimension::CommandSlots,
                None,
                None,
                NonZeroU64::new(1).unwrap(),
            )
            .unwrap(),
        ],
    )
    .unwrap();
    let attempted = event(
        3,
        &job.job_id,
        None,
        CodingEventKind::BudgetReserved {
            reservation: Box::new(reservation),
        },
    );
    assert_eq!(
        store
            .reserve_coding_budget_fenced(attempted, Some(&fresh))
            .await
            .unwrap_err(),
        StoreError::Conflict
    );
    store.close().await;
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn paused_job_resumes_only_with_fresh_fenced_authority() {
    let root = temp_root("paused-resume");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let job = create_job(&store, "paused-resume").await;
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
    let old = CodingRuntimeAuthority {
        epoch: 1,
        policy_digest: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
    };
    store.bind_coding_runtime(&job.job_id, &old).await.unwrap();
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
            .renew_coding_runtime(&job.job_id, Some(&old), &old, 2)
            .await
            .unwrap_err(),
        StoreError::Conflict
    );
    assert_eq!(
        store
            .append_coding_event(event(
                3,
                &job.job_id,
                None,
                CodingEventKind::StateChanged {
                    state: JobState::Running
                }
            ))
            .await
            .unwrap_err(),
        StoreError::Conflict
    );
    let fresh = CodingRuntimeAuthority {
        epoch: 2,
        ..old.clone()
    };
    store
        .renew_coding_runtime(&job.job_id, Some(&old), &fresh, 2)
        .await
        .unwrap();
    let state = store.coding_recovery_status(&job.job_id).await.unwrap();
    assert_eq!(state.state, JobState::Running);
    assert_eq!(state.authority, Some(fresh));
    assert_eq!(state.last_sequence, 3);
    store.close().await;
    std::fs::remove_dir_all(root).unwrap();
}
