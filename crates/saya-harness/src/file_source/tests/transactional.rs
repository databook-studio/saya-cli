use super::*;

#[test]
fn same_bytes_stage_to_the_same_sha256() {
    let root = temp_root("sha");
    let bytes = b"a,b\n1,2\n";
    write_bytes(&root.join("in").join("one.csv"), bytes);
    write_bytes(&root.join("in").join("two.csv"), bytes);
    let staged_one = stage_csv(
        &root.join("in").join("one.csv"),
        &root.join("d1"),
        CsvStageOptions {
            delimiter: None,
            header: true,
        },
    )
    .expect("first staging");
    let staged_two = stage_csv(
        &root.join("in").join("two.csv"),
        &root.join("d2"),
        CsvStageOptions {
            delimiter: None,
            header: true,
        },
    )
    .expect("second staging");
    let restaged = stage_csv(
        &root.join("in").join("one.csv"),
        &root.join("d3"),
        CsvStageOptions {
            delimiter: None,
            header: true,
        },
    )
    .expect("third staging");
    assert_eq!(staged_one.sha256, staged_two.sha256);
    assert_eq!(staged_one.sha256, restaged.sha256);
    assert_eq!(staged_one.sha256.len(), 64);
    assert!(
        staged_one
            .sha256
            .chars()
            .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
    );
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn staging_never_opens_the_parents_other_files() {
    let root = temp_root("sibling");
    let dir = root.join("in");
    write_bytes(&dir.join("data.csv"), b"a,b\n1,2\n");
    let locked = dir.join("locked.csv");
    fs::write(&locked, b"unreadable sibling").expect("sibling fixture");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("lock sibling");
    let staged = stage_csv(
        &dir.join("data.csv"),
        &root.join("dest"),
        CsvStageOptions {
            delimiter: None,
            header: true,
        },
    );
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).expect("unlock sibling");
    assert!(
        staged.is_ok(),
        "staging succeeds beside an unreadable sibling"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn elapsed_deadline_rolls_back_and_leaves_no_file() {
    let root = temp_root("deadline");
    let dest = root.join("dest");
    fs::create_dir_all(&dest).expect("destination");
    let source = read::SourceRead {
        bytes: b"a,b\n1,2\n".to_vec(),
        sha256: "0".repeat(64),
        file_name: "data.csv".to_owned(),
        stem: "data".to_owned(),
        size: 8,
    };
    let error = csv_stage::write_staged(
        &dest,
        "data",
        &["a".to_owned(), "b".to_owned()],
        &[vec!["1".to_owned(), "2".to_owned()]],
        &source,
        Instant::now(),
    )
    .expect_err("elapsed deadline refuses");
    assert!(matches!(error, StageError::Timeout));
    assert_dest_empty(&dest);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn failed_database_write_leaves_no_file() {
    let root = temp_root("dbfail");
    let dest = root.join("dest");
    fs::create_dir_all(&dest).expect("destination");
    let source = read::SourceRead {
        bytes: b"a,b\n1,2\n".to_vec(),
        sha256: "0".repeat(64),
        file_name: "data.csv".to_owned(),
        stem: "data".to_owned(),
        size: 8,
    };
    // An empty column list produces invalid `CREATE TABLE` SQL, so the write
    // fails at the database layer after the temp file already exists — the
    // cleanup guard must leave nothing behind.
    let error = csv_stage::write_staged(
        &dest,
        "data",
        &[],
        &[],
        &source,
        Instant::now() + STAGE_TIMEOUT,
    )
    .expect_err("empty column list refused");
    assert!(matches!(error, StageError::Database));
    assert_dest_empty(&dest);
    let _ = fs::remove_dir_all(root);
}
