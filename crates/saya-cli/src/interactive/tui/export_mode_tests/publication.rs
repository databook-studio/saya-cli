use super::*;

/// The writer refuses an existing destination unless `--overwrite` is
/// given, and the refusal leaves the existing file byte-for-byte.
#[test]
fn export_refuses_existing_destination_without_overwrite() {
    let dir = temp_dir("refuse-existing");
    let path = dir.join("out.csv");
    std::fs::write(&path, "keep-me").expect("seed the destination");
    let error = write_result_overwrite(&sample_result(), &path, false).unwrap_err();
    assert!(
        error.contains("exists; add --overwrite"),
        "the refusal names the flag: {error}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "keep-me",
        "the existing file is untouched"
    );
    assert_no_temp_files(&dir);
}

/// With `--overwrite` the export replaces the existing file in one rename:
/// the content is fully the new export, never a mix.
#[test]
fn export_overwrite_replaces_atomically() {
    let dir = temp_dir("overwrite");
    let path = dir.join("out.csv");
    std::fs::write(&path, "old").expect("seed the destination");
    let written = write_result_overwrite(&sample_result(), &path, true).unwrap();
    assert_eq!(written, 2);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "id,name\n1,alice\n2,\"bob, jr\"\n",
        "the export fully replaced the old content"
    );
    assert_no_temp_files(&dir);
}

/// A symlink destination is refused even with `--overwrite` — the export
/// never writes through a link to somewhere else.
#[cfg(unix)]
#[test]
fn export_refuses_symlink_destination() {
    let dir = temp_dir("symlink");
    let real = dir.join("real.csv");
    std::fs::write(&real, "real").expect("seed the link target");
    let link = dir.join("link.csv");
    std::os::unix::fs::symlink(&real, &link).expect("symlink");
    let error = write_result_overwrite(&sample_result(), &link, false).unwrap_err();
    assert!(
        error.contains("symlink"),
        "the refusal names the link: {error}"
    );
    assert_eq!(
        std::fs::read_to_string(&real).unwrap(),
        "real",
        "the link target is untouched"
    );
    assert!(
        std::fs::symlink_metadata(&link)
            .expect("link still there")
            .file_type()
            .is_symlink(),
        "the link itself is untouched"
    );
}

/// An encoding failure (here: the oversize ceiling, injected through the
/// test-sized helper) must leave an existing destination byte-for-byte and
/// create nothing.
#[test]
fn failed_export_preserves_destination() {
    let dir = temp_dir("failed-oversize");
    let path = dir.join("out.csv");
    std::fs::write(&path, "keep-me").expect("seed the destination");
    let big = QueryResult {
        columns: vec!["t".to_string()],
        rows: vec![serde_json::json!(["x".repeat(64)])],
        row_count: 1,
        truncated: false,
        executed_sql: "SELECT t".to_string(),
    };
    // Over the (tiny, injected) ceiling even with --overwrite: the failure
    // is the ceiling, and the destination survives it.
    let error = write_result_with_ceiling(&big, &path, true, 16).unwrap_err();
    assert_eq!(error, "export larger than 32 MiB");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "keep-me",
        "the existing file is untouched on failure"
    );
    // A fresh destination over the ceiling creates nothing — for CSV and,
    // through the serialiser's own writer, for JSON alike.
    let fresh = dir.join("fresh.csv");
    write_result_with_ceiling(&big, &fresh, false, 16).unwrap_err();
    assert!(!fresh.exists(), "no partial file on failure");
    let fresh_json = dir.join("fresh.json");
    write_result_with_ceiling(&big, &fresh_json, false, 16).unwrap_err();
    assert!(!fresh_json.exists(), "no partial JSON file on failure");
    assert_no_temp_files(&dir);
}

/// A publish failure after the guard (here: an unwritable directory, so
/// the private temp file cannot be created) leaves the existing
/// destination byte-for-byte and no temp file behind.
#[cfg(unix)]
#[test]
fn failed_publish_in_unwritable_dir_leaves_no_temp() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir("failed-publish");
    let path = dir.join("out.csv");
    std::fs::write(&path, "keep-me").expect("seed the destination");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555))
        .expect("chmod the dir read-only");
    let error = write_result_overwrite(&sample_result(), &path, true).unwrap_err();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
        .expect("restore the dir for cleanup");
    assert!(
        error.contains("temporary"),
        "the failure names the staging step: {error}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "keep-me",
        "the existing file is untouched when staging fails"
    );
    assert_no_temp_files(&dir);
}
