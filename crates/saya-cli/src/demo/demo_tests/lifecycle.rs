#[tokio::test]
async fn reuse_keeps_the_database_file() {
    let dir = temp_demo_dir("reuse");
    assert!(matches!(
        fixture::ensure(&dir, false).await.unwrap(),
        FixtureOutcome::Built
    ));
    let before = fs::metadata(fixture::db_path(&dir))
        .unwrap()
        .modified()
        .unwrap();
    assert!(matches!(
        fixture::ensure(&dir, false).await.unwrap(),
        FixtureOutcome::Reused
    ));
    let after = fs::metadata(fixture::db_path(&dir))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(before, after, "reuse must not rewrite the database file");
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn reset_rebuilds_the_fixture() {
    let dir = temp_demo_dir("reset");
    fixture::ensure(&dir, false).await.unwrap();
    assert!(matches!(
        fixture::ensure(&dir, true).await.unwrap(),
        FixtureOutcome::Built
    ));
    let _ = fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn missing_or_corrupt_database_is_rebuilt() {
    let dir = temp_demo_dir("corrupt");
    fs::create_dir_all(&dir).unwrap();
    fs::write(fixture::db_path(&dir), b"not a sqlite database").unwrap();
    assert!(
        matches!(
            fixture::ensure(&dir, false).await.unwrap(),
            FixtureOutcome::Built
        ),
        "a corrupt or wrong-version file is rebuilt, not reused"
    );
    assert!(matches!(
        fixture::ensure(&dir, false).await.unwrap(),
        FixtureOutcome::Reused
    ));
    let _ = fs::remove_dir_all(&dir);
}
