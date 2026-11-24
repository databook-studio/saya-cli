// The run's own rewind answers with the files it changed: the ones it wrote
// back and the ones it removed to match the checkpoint. A key renamed there
// would otherwise read as "nothing changed".

use super::changed_paths;

#[test]
fn a_restore_result_names_every_file_it_changed() {
    let result = serde_json::json!({
        "checkpoint": 2,
        "restored_paths": [{"path": "notes.txt", "sha256": "ab", "size": 3}],
        "removed_paths": ["scratch.md"],
    });
    assert_eq!(
        changed_paths(&result),
        vec!["notes.txt".to_owned(), "scratch.md".to_owned()]
    );
    assert!(
        changed_paths(&serde_json::json!({"checkpoint": 2})).is_empty(),
        "a bare result names no file"
    );
}
