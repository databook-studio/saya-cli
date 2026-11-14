#[tokio::test]
async fn connections_toml_is_a_single_private_demo_profile() {
    let dir = temp_demo_dir("connections");
    fixture::ensure(&dir, false).await.unwrap();
    let connections = fs::read_to_string(fixture::connections_path(&dir)).unwrap();
    let parsed = ConnectionsFile::from_toml(&connections)
        .unwrap_or_else(|error| panic!("saya must parse the written connections file: {error}"));
    let expected = DatabaseProfile::Sqlite {
        path: fixture::db_path(&dir).display().to_string(),
        read_only: true,
    };
    let profiles: Vec<(String, DatabaseProfile)> = parsed.profiles.into_iter().collect();
    assert_eq!(
        profiles,
        vec![("demo".to_owned(), expected)],
        "exactly one demo profile pointing at the demo database"
    );
    #[cfg(unix)]
    {
        let conn_mode = fs::metadata(fixture::connections_path(&dir))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(conn_mode, 0o600, "the connections file is 0600");
        let db_mode = fs::metadata(fixture::db_path(&dir))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(db_mode, 0o600, "the demo database is 0600");
        let dir_mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700, "the demo directory is 0700");
    }
    let _ = fs::remove_dir_all(&dir);
}

/// A symlink at the connections path is refused: the write must never follow
/// it, so the outside file keeps its bytes and the link itself stays a link.
#[tokio::test]
#[cfg(unix)]
async fn symlinked_connections_file_is_refused() {
    use std::os::unix::fs::symlink;

    let dir = temp_demo_dir("connections-symlink");
    fs::create_dir_all(&dir).unwrap();
    let outside = temp_demo_dir("connections-symlink-target");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("elsewhere.toml"), "keep = true\n").unwrap();
    symlink(
        outside.join("elsewhere.toml"),
        fixture::connections_path(&dir),
    )
    .unwrap();

    let error = match fixture::ensure(&dir, false).await {
        Err(error) => error,
        Ok(_) => panic!("a symlinked connections file must be refused"),
    };
    assert!(
        error.contains("refusing"),
        "the error names the refusal, got: {error}"
    );
    assert_eq!(
        fs::read_to_string(outside.join("elsewhere.toml")).unwrap(),
        "keep = true\n",
        "the symlink target is untouched"
    );
    assert!(
        fixture::connections_path(&dir)
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink(),
        "the symlink itself is not replaced by a regular file"
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(outside);
}

/// A directory sitting at the connections path is refused, not replaced, and
/// no staged temp file is left behind.
#[tokio::test]
async fn directory_at_connections_path_is_refused() {
    let dir = temp_demo_dir("connections-dir");
    fs::create_dir_all(fixture::connections_path(&dir)).unwrap();

    let error = match fixture::ensure(&dir, false).await {
        Err(error) => error,
        Ok(_) => panic!("a directory at the connections path must be refused"),
    };
    assert!(
        error.contains("refusing"),
        "the error names the refusal, got: {error}"
    );
    assert!(
        fixture::connections_path(&dir).is_dir(),
        "the directory is untouched"
    );
    let leftovers: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp-"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "no staged temp file is left behind: {leftovers:?}"
    );
    let _ = fs::remove_dir_all(dir);
}
