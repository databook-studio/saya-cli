use super::{
    event_projection_storage::event,
    job_metadata_storage::{child, create_job, temp_root},
};
use saya_store::{SqliteStateStore, StoreError};
use saya_types::{AgentId, CodingEventKind, InvocationId, JobState};

async fn store_with_child(tag: &str) -> (SqliteStateStore, saya_store::CodingChildRecord) {
    let store = SqliteStateStore::new(temp_root(tag).join("state.sqlite3"));
    let job = create_job(&store, tag).await;
    let record = store
        .create_coding_child(child(job.job_id.clone(), tag))
        .await
        .unwrap();
    (store, record)
}

#[tokio::test]
async fn a_queued_child_is_removed_and_its_agent_can_be_registered_again() {
    let (store, record) = store_with_child("remove-queued").await;

    store
        .remove_unscheduled_coding_child(
            &record.job_id,
            &record.agent_id,
            &record.current_invocation_id,
        )
        .await
        .unwrap();

    let page = store
        .list_coding_children(&record.job_id, None, 10)
        .await
        .unwrap();
    assert!(page.items.is_empty());
    store
        .create_coding_child(child(record.job_id.clone(), "remove-queued"))
        .await
        .expect("the same agent and invocation register again");
}

#[tokio::test]
async fn a_removal_names_the_registration_it_undoes() {
    let (store, record) = store_with_child("remove-named").await;
    let unknown = AgentId::parse("agent-unknown").unwrap();
    let other = InvocationId::parse("invocation-other").unwrap();

    let missing = store
        .remove_unscheduled_coding_child(&record.job_id, &unknown, &record.current_invocation_id)
        .await;
    let changed = store
        .remove_unscheduled_coding_child(&record.job_id, &record.agent_id, &other)
        .await;

    assert!(matches!(missing, Err(StoreError::NotFound)), "{missing:?}");
    assert!(matches!(changed, Err(StoreError::Conflict)), "{changed:?}");
    let page = store
        .list_coding_children(&record.job_id, None, 10)
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1, "a refused removal removes nothing");
}

#[tokio::test]
async fn a_child_that_started_is_never_removed() {
    let (store, record) = store_with_child("remove-started").await;
    for (sequence, target) in [(1, None), (2, Some(&record))] {
        store
            .append_coding_event(event(
                sequence,
                &record.job_id,
                target,
                CodingEventKind::StateChanged {
                    state: JobState::Running,
                },
            ))
            .await
            .unwrap();
    }

    let result = store
        .remove_unscheduled_coding_child(
            &record.job_id,
            &record.agent_id,
            &record.current_invocation_id,
        )
        .await;

    assert!(matches!(result, Err(StoreError::Conflict)), "{result:?}");
    let page = store
        .list_coding_children(&record.job_id, None, 10)
        .await
        .unwrap();
    assert_eq!(page.items[0].state, JobState::Running);
}
