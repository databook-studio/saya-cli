use super::*;

// ---------------------------------------------------------------------------
// The bounds pin their scale, so a loosened bound is visible in the diff
// ---------------------------------------------------------------------------

#[test]
fn the_bounds_pin_their_scale() {
    assert_eq!(CHECKPOINT_VERSION, 1);
    assert_eq!(MAX_CHECKPOINT_RECORD_BYTES, 4 * 1024);
    assert_eq!(MAX_CHECKPOINT_FILE_BYTES, 8 * 1024 * 1024);
    assert_eq!(MAX_CHECKPOINT_SESSION_BYTES, 64 * 1024 * 1024);
    assert_eq!(MAX_CHECKPOINTS_PER_SESSION, 128);
    assert_eq!(MAX_CHECKPOINT_PATH_BYTES, 512);
}

// ---------------------------------------------------------------------------
// Valid records round-trip
// ---------------------------------------------------------------------------

#[test]
fn a_valid_record_in_each_state_round_trips() {
    for make in [prepared, committed, uncertain] {
        let record = make();
        let bytes = record.to_json_bytes().unwrap();
        assert_eq!(EditCheckpoint::from_json_bytes(&bytes).unwrap(), record);
    }
}

#[test]
fn a_create_edit_round_trips_with_an_absent_before_image() {
    let record = create_committed();
    let bytes = record.to_json_bytes().unwrap();
    assert_eq!(EditCheckpoint::from_json_bytes(&bytes).unwrap(), record);
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["before"], "absent");
    assert!(json.get("after").is_some());
}

/// The wire names are the short `edit`/`write`; the earlier names stay readable
/// so stored checkpoints still load.
#[test]
fn edit_sources_emit_the_short_wire_names_and_still_read_the_old_ones() {
    assert_eq!(
        serde_json::to_value(EditSource::WorkspaceEdit).unwrap(),
        serde_json::json!("edit")
    );
    assert_eq!(
        serde_json::to_value(EditSource::WorkspaceWrite).unwrap(),
        serde_json::json!("write")
    );
    assert_eq!(
        serde_json::to_value(EditSource::NotebookEdit).unwrap(),
        serde_json::json!("notebook_edit")
    );
    for (wire, expected) in [
        ("edit", EditSource::WorkspaceEdit),
        ("workspace_edit", EditSource::WorkspaceEdit),
        ("write", EditSource::WorkspaceWrite),
        ("workspace_write", EditSource::WorkspaceWrite),
        ("notebook_edit", EditSource::NotebookEdit),
    ] {
        assert_eq!(
            serde_json::from_value::<EditSource>(serde_json::json!(wire)).unwrap(),
            expected,
            "wire name {wire:?} must load"
        );
    }
}

// ---------------------------------------------------------------------------
// The version gate: unknown versions are refused, never downgraded
// ---------------------------------------------------------------------------

#[test]
fn an_unknown_version_is_refused_never_ignored() {
    for v in [0u32, 2] {
        let mut record = prepared();
        record.version = v;
        assert_eq!(
            EditCheckpoint::from_json_bytes(&bytes_of(&record)),
            Err(CheckpointError::UnsupportedVersion(v))
        );
        assert_eq!(
            record.validate(),
            Err(CheckpointError::UnsupportedVersion(v))
        );
    }
}

// ---------------------------------------------------------------------------
// Unknown fields and non-file sources are refused at every level
// ---------------------------------------------------------------------------

#[test]
fn unknown_fields_are_refused() {
    let top = format!(
        r#"{{"version":1,"id":"{ID}","session_id":"{SESSION}","path":"src/main.rs","source":"edit","state":"prepared","before":{{"present":{{"sha256":"{BEFORE_SHA256}","size":16}}}},"created_unix_ms":{CREATED},"updated_unix_ms":{CREATED},"bonus":1}}"#
    );
    assert_eq!(
        EditCheckpoint::from_json_bytes(top.as_bytes()),
        Err(CheckpointError::Malformed)
    );
    let nested = top.replace(r#""size":16"#, r#""size":16,"extra":1"#);
    assert_eq!(
        EditCheckpoint::from_json_bytes(nested.as_bytes()),
        Err(CheckpointError::Malformed)
    );
}

#[test]
fn a_subprocess_or_shell_source_is_refused_never_ignored() {
    let template = |source: &str| {
        format!(
            r#"{{"version":1,"id":"{ID}","session_id":"{SESSION}","path":"src/main.rs","source":"{source}","state":"prepared","before":{{"present":{{"sha256":"{BEFORE_SHA256}","size":16}}}},"created_unix_ms":{CREATED},"updated_unix_ms":{CREATED}}}"#
        )
    };
    for source in ["subprocess", "shell"] {
        assert_eq!(
            EditCheckpoint::from_json_bytes(template(source).as_bytes()),
            Err(CheckpointError::Malformed),
            "a `{source}` effect is never representable as a checkpoint"
        );
    }
    EditCheckpoint::from_json_bytes(template("edit").as_bytes())
        .expect("the file-tool twin is the only representable source");
}
