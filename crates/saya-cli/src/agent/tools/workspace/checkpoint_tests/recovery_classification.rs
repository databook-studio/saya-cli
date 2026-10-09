use super::*;

#[tokio::test]
async fn a_crash_after_the_commit_leaves_an_uncertain_record_on_recovery() {
    let sandbox = Sandbox::new("crash");
    let store = sandbox.sessions_store();
    let recorder = CheckpointRecorder::new(
        store.clone(),
        SESSION.into(),
        Arc::new(AtomicBool::new(true)),
    );

    // A capture prepared, the file commit landed, then the process died
    // before `finish_ok`: the record stays `Prepared` until recovery runs.
    let capture = recorder.begin(
        "notes.txt",
        EditSource::WorkspaceEdit,
        Some(b"before".as_slice()),
    );
    drop(capture);

    let report = store.recover_checkpoints(SESSION).expect("recovery runs");
    assert_eq!(report.marked_uncertain, 1);
    let records = store.list_checkpoints(SESSION).expect("list");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].state, CheckpointState::Uncertain);
}

#[test]
fn edit_refusals_classify_clean_only_when_nothing_was_written() {
    let clean = [
        ToolError::WorkspaceEditMoved {
            path: "p".into(),
            expected_size: None,
            current_size: 1,
            expected_digest: None,
            current_digest: "d".into(),
        },
        ToolError::WorkspaceAppendOffset {
            path: "p".into(),
            expected_offset: 0,
            current_size: 1,
            current_digest: "d".into(),
        },
    ];
    for error in &clean {
        assert_eq!(classify_edit(error), RefusalClass::Clean);
    }
    let ambiguous = [
        ToolError::WorkspaceEdit("harness failure: identity changed after a rename".into()),
        ToolError::WorkspaceEditNotText { path: "p".into() },
        ToolError::WorkspaceWrite("unexpected channel".into()),
    ];
    for error in &ambiguous {
        assert_eq!(classify_edit(error), RefusalClass::Ambiguous);
    }
}

#[test]
fn write_refusals_classify_clean_only_for_the_constant_preconditions() {
    let clean = [
        ToolError::WorkspaceWrite(FILE_EXISTS_REQUIRES_DIGEST.to_owned()),
        ToolError::WorkspaceWrite(FILE_CHANGED_SINCE_OBSERVED.to_owned()),
        ToolError::WorkspaceWrite(MISSING_FILE_NOT_A_REPLACE.to_owned()),
    ];
    for error in &clean {
        assert_eq!(classify_write(error), RefusalClass::Clean);
    }
    let ambiguous = [
        ToolError::WorkspaceWrite("identity changed after a rename".into()),
        ToolError::WorkspaceEditMoved {
            path: "p".into(),
            expected_size: None,
            current_size: 1,
            expected_digest: None,
            current_digest: "d".into(),
        },
        ToolError::WorkspaceEdit("unexpected channel".into()),
    ];
    for error in &ambiguous {
        assert_eq!(classify_write(error), RefusalClass::Ambiguous);
    }
}

#[test]
fn quota_and_screen_refusals_map_to_model_safe_texts() {
    assert_eq!(
        quota_refusal_text(CheckpointRefusal::FileTooLarge),
        "recovery is enabled but this edit cannot be checkpointed (file too large); \
         the file was not changed"
    );
    assert_eq!(
        quota_refusal_text(CheckpointRefusal::TooManyCheckpoints),
        "recovery is enabled but this edit cannot be checkpointed (too many checkpoints); \
         the file was not changed"
    );
    assert_eq!(
        quota_refusal_text(CheckpointRefusal::SessionBytesExceeded),
        "recovery is enabled but this edit cannot be checkpointed \
         (session recovery storage is full); the file was not changed"
    );
    assert_eq!(
        screen_class_text(ScreenRefusal::CredentialPath),
        "credential path"
    );
    assert_eq!(
        screen_class_text(ScreenRefusal::SecretContent),
        "secret-shaped content"
    );
    assert_eq!(
        screen_class_text(ScreenRefusal::DatabaseExport),
        "database export"
    );
}
