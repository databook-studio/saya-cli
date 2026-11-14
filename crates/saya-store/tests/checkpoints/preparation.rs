use super::*;

#[test]
fn round_trip_keeps_bytes_exact_through_prepare_commit_list_and_read() {
    let (state, store) = ck_store("round-trip");
    assert_eq!(
        store.list_checkpoints(SESSION).unwrap(),
        Vec::<EditCheckpoint>::new()
    );
    let before_bytes: &[u8] = b"line one\r\n\xff\xfe\x00not utf8 \xc3\x28\r\n";
    let record = prepare(&store, SESSION, "notes/a.txt", Some(before_bytes), 1_000).unwrap();
    assert_eq!(record.session_id, SESSION);
    assert_eq!(record.path, "notes/a.txt");
    assert_eq!(record.state, CheckpointState::Prepared);
    assert_eq!(record.before, BeforeImage::Present(after_of(before_bytes)));
    assert_eq!(record.created_unix_ms, 1_000);
    assert_eq!(record.updated_unix_ms, 1_000);

    let after_bytes = b"line one edited\r\n";
    let committed = store
        .commit_checkpoint(SESSION, record.id.as_str(), after_of(after_bytes), 2_000)
        .unwrap();
    assert_eq!(committed.state, CheckpointState::Committed);
    assert_eq!(committed.after, Some(after_of(after_bytes)));
    assert_eq!(committed.updated_unix_ms, 2_000);

    let listed = store.list_checkpoints(SESSION).unwrap();
    assert_eq!(listed, vec![committed]);
    assert_eq!(
        store
            .read_before_image(SESSION, record.id.as_str())
            .unwrap(),
        before_bytes.to_vec()
    );
    assert_eq!(
        store.checkpoint_usage(SESSION).unwrap(),
        CheckpointUsage {
            edits: 1,
            retained_bytes: before_bytes.len() as u64
        }
    );
    let _ = fs::remove_dir_all(state);
}
#[test]
fn an_absent_before_records_a_create_and_writes_no_before_file() {
    let (state, store) = ck_store("absent");
    let record = prepare(&store, SESSION, "new/file.txt", None, 1_000).unwrap();
    assert_eq!(record.before, BeforeImage::Absent);
    let image = ck_dir(&state, SESSION).join(format!("{}.before", record.id.as_str()));
    assert!(!image.exists());
    let error = store
        .read_before_image(SESSION, record.id.as_str())
        .unwrap_err();
    assert_eq!(error, CheckpointStoreError::NotFound);
    assert_eq!(
        store.checkpoint_usage(SESSION).unwrap(),
        CheckpointUsage {
            edits: 1,
            retained_bytes: 0
        }
    );
    store
        .commit_checkpoint(SESSION, record.id.as_str(), after_of(b"created"), 2_000)
        .unwrap();
    let _ = fs::remove_dir_all(state);
}
#[test]
fn a_disabled_policy_writes_nothing_at_all() {
    let (state, store) = ck_store("disabled");
    let disabled = CheckpointPolicy { enabled: false };
    let error = store
        .prepare_checkpoint(
            SESSION,
            &disabled,
            NewCheckpoint {
                path: "a.txt",
                source: EditSource::WorkspaceEdit,
                before: Some(b"x"),
            },
            1_000,
        )
        .unwrap_err();
    assert_eq!(error, CheckpointStoreError::Disabled);
    assert!(
        !state.join("sessions").exists(),
        "not even the sessions root or the session dir is written"
    );
    let _ = fs::remove_dir_all(state);
}
#[test]
fn ids_are_ck_hex_and_unique_across_200_prepares() {
    let (state, store) = ck_store("ids");
    let mut ids = HashSet::new();
    for session in ["s1", "s2", "s3", "s4"] {
        for i in 0..50 {
            let path = format!("f{i}.txt");
            let record = prepare(&store, session, &path, Some(b"x"), 1_000).unwrap();
            let id = record.id.as_str();
            assert_eq!(id.len(), 35, "ck- plus 32 hex: {id}");
            assert!(id.starts_with("ck-"));
            assert!(
                id[3..]
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            );
            ids.insert(id.to_owned());
        }
    }
    assert_eq!(ids.len(), 200, "every minted id is unique");
    let _ = fs::remove_dir_all(state);
}
#[test]
fn the_128th_checkpoint_succeeds_and_the_129th_is_refused_before_any_write() {
    let (state, store) = ck_store("count-quota");
    for i in 0..128 {
        let path = format!("f{i}.txt");
        prepare(&store, SESSION, &path, Some(b"x"), 1_000).unwrap();
    }
    assert_eq!(store.checkpoint_usage(SESSION).unwrap().edits, 128);
    let dir = ck_dir(&state, SESSION);
    let before_count = entry_count(&dir);
    let error = prepare(&store, SESSION, "f129.txt", Some(b"y"), 1_000).unwrap_err();
    assert_eq!(
        error,
        CheckpointStoreError::Refused(CheckpointRefusal::TooManyCheckpoints)
    );
    assert_eq!(entry_count(&dir), before_count, "nothing written at quota");
    assert_eq!(store.checkpoint_usage(SESSION).unwrap().edits, 128);
    let _ = fs::remove_dir_all(state);
}
#[test]
fn the_file_cap_boundary_is_exact() {
    let (state, store) = ck_store("file-cap");
    let cap = MAX_CHECKPOINT_FILE_BYTES as usize;
    let at_cap = vec![0x61u8; cap];
    prepare(&store, SESSION, "big.txt", Some(&at_cap), 1_000).unwrap();
    let over = vec![0x61u8; cap + 1];
    let dir = ck_dir(&state, SESSION);
    let error = prepare(&store, SESSION, "big2.txt", Some(&over), 1_000).unwrap_err();
    assert_eq!(
        error,
        CheckpointStoreError::Refused(CheckpointRefusal::FileTooLarge)
    );
    assert_eq!(
        entry_count(&dir),
        2,
        "only the at-cap checkpoint's record and image exist"
    );
    assert_eq!(
        store.checkpoint_usage(SESSION).unwrap(),
        CheckpointUsage {
            edits: 1,
            retained_bytes: cap as u64
        }
    );
    let _ = fs::remove_dir_all(state);
}
#[test]
fn the_session_bytes_boundary_is_exact_at_the_cap() {
    let (state, store) = ck_store("session-bytes");
    let cap = MAX_CHECKPOINT_FILE_BYTES as usize;
    let buffer = vec![0x61u8; cap];
    for i in 0..8 {
        let path = format!("f{i}.txt");
        prepare(&store, SESSION, &path, Some(&buffer), 1_000).unwrap_or_else(|error| {
            panic!(
                "checkpoint {i} (total {} MiB) must be admitted: {error:?}",
                (i + 1) * cap / 1024 / 1024
            )
        });
    }
    assert_eq!(
        store.checkpoint_usage(SESSION).unwrap(),
        CheckpointUsage {
            edits: 8,
            retained_bytes: MAX_CHECKPOINT_SESSION_BYTES
        }
    );
    let dir = ck_dir(&state, SESSION);
    let error = prepare(&store, SESSION, "one-more.txt", Some(b"z"), 1_000).unwrap_err();
    assert_eq!(
        error,
        CheckpointStoreError::Refused(CheckpointRefusal::SessionBytesExceeded)
    );
    assert_eq!(
        entry_count(&dir),
        16,
        "nothing written past the session cap"
    );
    let _ = fs::remove_dir_all(state);
}
#[test]
fn invalid_record_paths_are_refused_before_any_write() {
    let (state, store) = ck_store("paths");
    for path in ["", "../x", "/abs.txt", "C:\\x", "a//b", "src/"] {
        let error = prepare(&store, SESSION, path, Some(b"x"), 1_000).unwrap_err();
        assert_eq!(error, CheckpointStoreError::Store(StoreError::Invalid));
        assert!(
            !state.join("sessions").join(SESSION).exists(),
            "path {path:?} refused before any write"
        );
    }
    let _ = fs::remove_dir_all(state);
}
