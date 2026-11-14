use super::*;

/// The snapshot honours a `.json` destination: the captured rows exported
/// as the same pretty JSON objects the re-run path writes.
#[test]
fn snapshot_export_writes_json_exactly() {
    let dir = temp_dir("snapshot-json");
    let path = dir.join("snap.json");
    let mut transcript = Transcript::new();
    let mut state = SessionState::new("test", None, "model");
    let captured = captured_of(sample_result());
    apply_export_action(
        snapshot_request(&path),
        &mut transcript,
        &mut state,
        &mut None,
        &captured,
    );
    let written = std::fs::read_to_string(&path).expect("snapshot wrote the file");
    assert_eq!(
        written,
        "[\n  {\n    \"id\": 1,\n    \"name\": \"alice\"\n  },\n  {\n    \"id\": 2,\n    \"name\": \"bob, jr\"\n  }\n]",
        "the JSON export is the captured rows, pretty-printed: {written}"
    );
}

/// The production ceiling is the 32 MiB the failure message names.
#[test]
fn max_export_ceiling_is_32_mib() {
    assert_eq!(MAX_EXPORT_BYTES, 32 * 1024 * 1024);
}
