use super::*;

#[test]
fn commit_is_idempotent_on_the_same_after_and_corrupt_on_a_different_one() {
    let (state, store) = ck_store("commit");
    let record = prepare(&store, SESSION, "a.txt", Some(b"v1"), 1_000).unwrap();
    let first = after_of(b"v2");
    let committed = store
        .commit_checkpoint(SESSION, record.id.as_str(), first.clone(), 2_000)
        .unwrap();
    let again = store
        .commit_checkpoint(SESSION, record.id.as_str(), first.clone(), 3_000)
        .unwrap();
    assert_eq!(
        again, committed,
        "repeating the same after is idempotent, updated unchanged"
    );
    let error = store
        .commit_checkpoint(SESSION, record.id.as_str(), after_of(b"v3"), 4_000)
        .unwrap_err();
    assert_eq!(error, CheckpointStoreError::Corrupt);
    assert_eq!(store.list_checkpoints(SESSION).unwrap(), vec![committed]);
    assert_eq!(
        store
            .commit_checkpoint(
                SESSION,
                "ck-00000000000000000000000000000000",
                first.clone(),
                5_000
            )
            .unwrap_err(),
        CheckpointStoreError::NotFound
    );
    assert_eq!(
        store
            .commit_checkpoint(SESSION, "garbage", first, 5_000)
            .unwrap_err(),
        CheckpointStoreError::NotFound
    );
    let _ = fs::remove_dir_all(state);
}
#[test]
fn mark_uncertain_is_idempotent_and_refuses_committed_records() {
    let (state, store) = ck_store("uncertain");
    let record = prepare(&store, SESSION, "a.txt", Some(b"v1"), 1_000).unwrap();
    let marked = store
        .mark_checkpoint_uncertain(SESSION, record.id.as_str(), 2_000)
        .unwrap();
    assert_eq!(marked.state, CheckpointState::Uncertain);
    assert_eq!(marked.updated_unix_ms, 2_000);
    let again = store
        .mark_checkpoint_uncertain(SESSION, record.id.as_str(), 3_000)
        .unwrap();
    assert_eq!(again, marked, "the uncertain mark is idempotent");
    assert_eq!(
        store
            .commit_checkpoint(SESSION, record.id.as_str(), after_of(b"v2"), 4_000)
            .unwrap_err(),
        CheckpointStoreError::Corrupt
    );

    let other = prepare(&store, SESSION, "b.txt", Some(b"v1"), 1_000).unwrap();
    store
        .commit_checkpoint(SESSION, other.id.as_str(), after_of(b"v2"), 2_000)
        .unwrap();
    assert_eq!(
        store
            .mark_checkpoint_uncertain(SESSION, other.id.as_str(), 3_000)
            .unwrap_err(),
        CheckpointStoreError::Corrupt
    );
    let _ = fs::remove_dir_all(state);
}
#[test]
fn a_tombstoned_session_refuses_checkpoint_writes_but_reads_still_work() {
    let (state, store) = ck_store("tombstone");
    plant_session(&store, SESSION);
    prepare(&store, SESSION, "a.txt", Some(b"kept"), 1_000).unwrap();
    store.begin_delete(SESSION).unwrap();
    let error = prepare(&store, SESSION, "b.txt", Some(b"new"), 2_000).unwrap_err();
    assert_eq!(error, CheckpointStoreError::Store(StoreError::Conflict));
    assert_eq!(
        store
            .commit_checkpoint(
                SESSION,
                "ck-00000000000000000000000000000000",
                after_of(b"v"),
                2_000
            )
            .unwrap_err(),
        CheckpointStoreError::Store(StoreError::Conflict)
    );
    assert_eq!(
        store
            .mark_checkpoint_uncertain(SESSION, "ck-00000000000000000000000000000000", 2_000)
            .unwrap_err(),
        CheckpointStoreError::Store(StoreError::Conflict)
    );
    assert_eq!(
        store.recover_checkpoints(SESSION).unwrap_err(),
        CheckpointStoreError::Store(StoreError::Conflict)
    );
    assert_eq!(
        store.list_checkpoints(SESSION).unwrap().len(),
        1,
        "reads still work on a tombstoned session"
    );
    let _ = fs::remove_dir_all(state);
}
