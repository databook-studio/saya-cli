//! Private checkpoint before-image storage (saya-store): the public-API
//! contract for opt-in recovery of direct file-tool edits. Checkpoints are
//! private recovery data — exact before-image bytes, never redacted — kept
//! under `<sessions>/<id>/checkpoints/` so the session's deletion removes
//! them. The gates asserted here: policy opt-in writes nothing when
//! disabled, quota admission refuses before any write, a failed prepare
//! leaves no half state, reads verify digest and size, crash recovery
//! converts `Prepared` to `Uncertain` and removes only orphan images and
//! staging temps, commit is idempotent on the same after-image, and
//! tombstoned sessions and symlinked paths are refused.

use saya_store::{
    CheckpointStoreError, DeleteStatus, FsSessionStore, NewCheckpoint, RecoveryReport,
    RedactedSession, SessionStore, StoreError,
};
use saya_types::{
    BeforeImage, CheckpointPolicy, CheckpointRefusal, CheckpointState, CheckpointUsage,
    EditCheckpoint, EditSource, FileVersion, MAX_CHECKPOINT_FILE_BYTES,
    MAX_CHECKPOINT_SESSION_BYTES,
};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, fs, path::Path, path::PathBuf};

const SESSION: &str = "worker";

fn temp_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "saya-store-checkpoints-{label}-{}",
        std::process::id()
    ))
}

/// Builds the store at `<tmp>/sessions` so `<tmp>` is the state dir whose
/// independent contents must survive a session deletion.
fn ck_store(label: &str) -> (PathBuf, FsSessionStore) {
    let state = temp_root(label);
    let _ = fs::remove_dir_all(&state);
    (state.clone(), FsSessionStore::new(state.join("sessions")))
}

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

fn ck_dir(state: &Path, session: &str) -> PathBuf {
    state.join("sessions").join(session).join("checkpoints")
}

fn entry_count(dir: &Path) -> usize {
    fs::read_dir(dir).unwrap().count()
}

fn policy() -> CheckpointPolicy {
    CheckpointPolicy { enabled: true }
}

fn digest_of(bytes: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(bytes);
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn after_of(bytes: &[u8]) -> FileVersion {
    FileVersion {
        sha256: digest_of(bytes),
        size: bytes.len() as u64,
    }
}

fn plant_session(store: &FsSessionStore, id: &str) {
    block_on(store.save(RedactedSession {
        id: id.to_owned(),
        ..Default::default()
    }))
    .unwrap();
}

fn prepare(
    store: &FsSessionStore,
    session: &str,
    path: &str,
    before: Option<&[u8]>,
    now: i64,
) -> Result<EditCheckpoint, CheckpointStoreError> {
    store.prepare_checkpoint(
        session,
        &policy(),
        NewCheckpoint {
            path,
            source: EditSource::WorkspaceEdit,
            before,
        },
        now,
    )
}

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

#[test]
fn crash_recovery_marks_prepared_uncertain_and_removes_only_orphans_and_temps() {
    let (state, store) = ck_store("crash");
    let bytes = b"crash survivor\r\n";
    let record = prepare(&store, SESSION, "a.txt", Some(bytes), 1_000).unwrap();
    let dir = ck_dir(&state, SESSION);
    let orphan = dir.join("ck-abababababababababababababababab.before");
    fs::write(&orphan, "orphan bytes").unwrap();
    let temp = dir.join(".x.tmp");
    fs::write(&temp, "staging leftover").unwrap();
    let foreign = dir.join("notes.txt");
    fs::write(&foreign, "keep me").unwrap();
    let garbage = dir.join("ck-cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd.json");
    fs::write(&garbage, "not json").unwrap();

    let listed = store.list_checkpoints(SESSION).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].state, CheckpointState::Prepared);
    let report = store.recover_checkpoints(SESSION).unwrap();
    assert_eq!(
        report,
        RecoveryReport {
            marked_uncertain: 1,
            orphans_removed: 2,
            skipped: 2
        }
    );
    assert!(!orphan.exists());
    assert!(!temp.exists());
    assert!(foreign.exists(), "foreign files are never touched");
    assert!(garbage.exists(), "unparsable records are never deleted");
    let listed = store.list_checkpoints(SESSION).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].state, CheckpointState::Uncertain);
    assert_eq!(
        store
            .read_before_image(SESSION, record.id.as_str())
            .unwrap(),
        bytes.to_vec()
    );
    assert_eq!(store.checkpoint_usage(SESSION).unwrap().edits, 1);
    let second = store.recover_checkpoints(SESSION).unwrap();
    assert_eq!(
        second,
        RecoveryReport {
            marked_uncertain: 0,
            orphans_removed: 0,
            skipped: 2
        }
    );
    let _ = fs::remove_dir_all(state);
}

#[cfg(unix)]
#[test]
fn a_failed_prepare_leaves_no_partial_state() {
    use std::os::unix::fs::PermissionsExt;

    struct Restore<'a> {
        path: &'a Path,
    }
    impl Drop for Restore<'_> {
        fn drop(&mut self) {
            let _ = fs::set_permissions(self.path, fs::Permissions::from_mode(0o700));
        }
    }

    let (state, store) = ck_store("read-only");
    prepare(&store, SESSION, "a.txt", Some(b"kept"), 1_000).unwrap();
    let dir = ck_dir(&state, SESSION);
    let before_count = entry_count(&dir);
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o500)).unwrap();
    let _guard = Restore { path: &dir };
    let error = prepare(&store, SESSION, "b.txt", Some(b"new"), 1_000).unwrap_err();
    assert!(matches!(error, CheckpointStoreError::Store(_)));
    assert_eq!(
        entry_count(&dir),
        before_count,
        "the failed prepare left no image, record, or staging temp"
    );
    drop(_guard);
    prepare(&store, SESSION, "b.txt", Some(b"new"), 1_000).unwrap();
    let _ = fs::remove_dir_all(state);
}

#[test]
fn read_before_image_refuses_tampered_bytes() {
    let (state, store) = ck_store("tamper");
    let bytes = b"tamper target bytes";
    let record = prepare(&store, SESSION, "a.txt", Some(bytes), 1_000).unwrap();
    let image = ck_dir(&state, SESSION).join(format!("{}.before", record.id.as_str()));
    let mut stored = fs::read(&image).unwrap();
    stored[0] ^= 0xff;
    fs::write(&image, &stored).unwrap();
    assert_eq!(
        store
            .read_before_image(SESSION, record.id.as_str())
            .unwrap_err(),
        CheckpointStoreError::Corrupt
    );

    let other = prepare(&store, SESSION, "b.txt", Some(bytes), 1_000).unwrap();
    let image = ck_dir(&state, SESSION).join(format!("{}.before", other.id.as_str()));
    let mut stored = fs::read(&image).unwrap();
    stored.pop();
    fs::write(&image, &stored).unwrap();
    assert_eq!(
        store
            .read_before_image(SESSION, other.id.as_str())
            .unwrap_err(),
        CheckpointStoreError::Corrupt
    );
    let _ = fs::remove_dir_all(state);
}

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

#[test]
fn invalid_session_ids_are_refused_for_every_operation() {
    let (state, store) = ck_store("invalid-session");
    let after = after_of(b"v");
    let id = "ck-00000000000000000000000000000000";
    for session in ["", "..", "a/b", "a\\b"] {
        assert!(matches!(
            prepare(&store, session, "a.txt", Some(b"x"), 1_000).unwrap_err(),
            CheckpointStoreError::Store(_)
        ));
        assert!(matches!(
            store.list_checkpoints(session).unwrap_err(),
            CheckpointStoreError::Store(_)
        ));
        assert!(matches!(
            store.checkpoint_usage(session).unwrap_err(),
            CheckpointStoreError::Store(_)
        ));
        assert!(matches!(
            store.read_before_image(session, id).unwrap_err(),
            CheckpointStoreError::Store(_)
        ));
        assert!(matches!(
            store.delete_checkpoint(session, id).unwrap_err(),
            CheckpointStoreError::Store(_)
        ));
        assert!(matches!(
            store
                .commit_checkpoint(session, id, after.clone(), 1_000)
                .unwrap_err(),
            CheckpointStoreError::Store(_)
        ));
        assert!(matches!(
            store.recover_checkpoints(session).unwrap_err(),
            CheckpointStoreError::Store(_)
        ));
        assert!(
            !state.join("sessions").join(session).exists(),
            "nothing is written for an invalid session id"
        );
    }
    let _ = fs::remove_dir_all(state);
}

#[cfg(unix)]
#[test]
fn symlinked_checkpoint_paths_are_refused_and_targets_untouched() {
    use std::os::unix::fs::symlink;

    let (state, store) = ck_store("symlink");
    let victim = state.join("victim");
    fs::create_dir_all(&victim).unwrap();
    fs::write(victim.join("keep.txt"), "precious").unwrap();

    fs::create_dir_all(state.join("sessions").join("s1")).unwrap();
    symlink(
        &victim,
        state.join("sessions").join("s1").join("checkpoints"),
    )
    .unwrap();
    let error = prepare(&store, "s1", "a.txt", Some(b"x"), 1_000).unwrap_err();
    assert_eq!(error, CheckpointStoreError::Store(StoreError::Invalid));
    assert_eq!(
        fs::read(victim.join("keep.txt")).unwrap(),
        b"precious".to_vec()
    );
    assert_eq!(entry_count(&victim), 1, "the link target gained nothing");

    symlink(&victim, state.join("sessions").join("s2")).unwrap();
    let error = prepare(&store, "s2", "a.txt", Some(b"x"), 1_000).unwrap_err();
    assert_eq!(error, CheckpointStoreError::Store(StoreError::Invalid));
    assert_eq!(entry_count(&victim), 1, "the link target gained nothing");
    let _ = fs::remove_dir_all(state);
}

#[test]
fn finishing_the_deletion_removes_the_checkpoints_dir_and_leaves_siblings_intact() {
    let (state, store) = ck_store("delete");
    fs::create_dir_all(state.join("runs")).unwrap();
    fs::write(state.join("state.sqlite3"), "db").unwrap();
    fs::write(state.join("input_history"), "history").unwrap();
    fs::write(state.join("runs/step"), "run").unwrap();
    plant_session(&store, "a");
    plant_session(&store, "other");
    let kept = prepare(&store, "other", "keep.txt", Some(b"sibling"), 1_000).unwrap();
    let gone_one = prepare(&store, "a", "one.txt", Some(b"first"), 1_000).unwrap();
    let gone_two = prepare(&store, "a", "two.txt", Some(b"second"), 1_000).unwrap();

    store.begin_delete("a").unwrap();
    assert_eq!(store.finish_delete("a").unwrap(), DeleteStatus::Complete);

    assert!(!state.join("sessions").join("a").exists());
    assert!(!state.join("sessions/a.json").exists());
    assert_eq!(
        fs::read(state.join("state.sqlite3")).unwrap(),
        b"db".to_vec()
    );
    assert_eq!(
        fs::read(state.join("input_history")).unwrap(),
        b"history".to_vec()
    );
    assert_eq!(fs::read(state.join("runs/step")).unwrap(), b"run".to_vec());
    assert_eq!(block_on(store.load("other")).unwrap().unwrap().id, "other");
    let listed = store.list_checkpoints("other").unwrap();
    assert_eq!(listed, vec![kept.clone()]);
    assert_eq!(
        store.read_before_image("other", kept.id.as_str()).unwrap(),
        b"sibling".to_vec()
    );
    assert_eq!(
        store
            .read_before_image("a", gone_one.id.as_str())
            .unwrap_err(),
        CheckpointStoreError::NotFound
    );
    assert_eq!(
        store
            .read_before_image("a", gone_two.id.as_str())
            .unwrap_err(),
        CheckpointStoreError::NotFound
    );
    let _ = fs::remove_dir_all(state);
}

#[test]
fn list_is_sorted_by_created_then_id() {
    let (state, store) = ck_store("order");
    prepare(&store, SESSION, "b.txt", Some(b"1"), 2_000).unwrap();
    prepare(&store, SESSION, "a.txt", Some(b"2"), 1_000).unwrap();
    prepare(&store, SESSION, "c.txt", Some(b"3"), 2_000).unwrap();
    let listed = store.list_checkpoints(SESSION).unwrap();
    assert_eq!(listed.len(), 3);
    assert_eq!(listed[0].path, "a.txt");
    assert!(
        listed
            .windows(2)
            .all(|pair| (pair[0].created_unix_ms, pair[0].id.as_str())
                < (pair[1].created_unix_ms, pair[1].id.as_str()))
    );
    let _ = fs::remove_dir_all(state);
}

#[test]
fn prepare_writes_the_bytes_before_the_record() {
    let (state, store) = ck_store("write-order");
    let record = prepare(&store, SESSION, "a.txt", Some(b"ordered"), 1_000).unwrap();
    let dir = ck_dir(&state, SESSION);
    let image_mtime = fs::symlink_metadata(dir.join(format!("{}.before", record.id.as_str())))
        .unwrap()
        .modified()
        .unwrap();
    let record_mtime = fs::symlink_metadata(dir.join(format!("{}.json", record.id.as_str())))
        .unwrap()
        .modified()
        .unwrap();
    assert!(
        image_mtime <= record_mtime,
        "the before-image bytes must land before the Prepared record: a crash in \
         between must leave an orphan image, never a record without its image"
    );
    let _ = fs::remove_dir_all(state);
}

#[test]
fn delete_checkpoint_removes_record_and_image_and_is_idempotent() {
    let (state, store) = ck_store("delete-ck");
    let record = prepare(&store, SESSION, "a.txt", Some(b"v1"), 1_000).unwrap();
    let dir = ck_dir(&state, SESSION);
    assert_eq!(entry_count(&dir), 2);
    store
        .delete_checkpoint(SESSION, record.id.as_str())
        .unwrap();
    assert_eq!(entry_count(&dir), 0);
    store
        .delete_checkpoint(SESSION, record.id.as_str())
        .unwrap();
    assert_eq!(
        store.checkpoint_usage(SESSION).unwrap(),
        CheckpointUsage::default()
    );
    assert_eq!(
        store
            .read_before_image(SESSION, record.id.as_str())
            .unwrap_err(),
        CheckpointStoreError::NotFound
    );
    // Deleting an unknown id is the same idempotent no-op; only a
    // malformed id is refused.
    store
        .delete_checkpoint(SESSION, "ck-00000000000000000000000000000000")
        .unwrap();
    assert_eq!(
        store.delete_checkpoint(SESSION, "garbage").unwrap_err(),
        CheckpointStoreError::NotFound
    );
    let _ = fs::remove_dir_all(state);
}
