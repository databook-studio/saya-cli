use super::*;

#[test]
fn screening_refuses_credential_paths_and_writes_nothing() {
    let (state, store) = ck_store("screen-paths");
    for path in [
        ".env",
        ".env.local",
        "config/.ssh/id_rsa",
        "deploy/credentials.json",
        "keys/server.pem",
        "app/secrets.toml",
        "AUTH.JSON",
        "a\\.aws\\credentials",
    ] {
        let error = prepare(&store, SESSION, path, Some(b"content"), 1_000).unwrap_err();
        assert_eq!(
            error,
            CheckpointStoreError::Screened(ScreenRefusal::CredentialPath),
            "path {path:?}"
        );
        assert!(
            !state.join("sessions").exists(),
            "path {path:?} refused before any write"
        );
    }
    let _ = fs::remove_dir_all(state);
}
#[test]
fn screening_admits_ordinary_source_paths_and_checkpoints_normally() {
    let (state, store) = ck_store("screen-admit");
    for (path, before) in [
        ("src/main.rs", Some(b"fn main() {}\n".as_slice())),
        ("README.md", Some(b"# readme\n".as_slice())),
        ("docs/auth_flow.md", Some(b"auth flow\n".as_slice())),
        ("config.toml", Some(b"[package]\n".as_slice())),
        (".gitignore", Some(b"target\n".as_slice())),
        ("Cargo.toml", Some(b"[package]\n".as_slice())),
        ("src/token_bucket.rs", Some(b"struct Bucket;\n".as_slice())),
        ("notes.txt", Some(b"plain text\n".as_slice())),
        ("empty.txt", Some(&b""[..])),
    ] {
        prepare(&store, SESSION, path, before, 1_000)
            .unwrap_or_else(|error| panic!("path {path:?} must be admitted: {error:?}"));
    }
    let record = prepare(&store, SESSION, "notes.txt", Some(b"round trip"), 2_000).unwrap();
    store
        .commit_checkpoint(SESSION, record.id.as_str(), after_of(b"edited"), 3_000)
        .unwrap();
    assert_eq!(
        store
            .read_before_image(SESSION, record.id.as_str())
            .unwrap(),
        b"round trip".to_vec()
    );
    let _ = fs::remove_dir_all(state);
}
#[test]
fn screening_refuses_secret_shaped_content() {
    let (state, store) = ck_store("screen-content");
    for content in [
        b"API_KEY=abc123\n".as_slice(),
        b"password=hunter2\n".as_slice(),
        b"Authorization: Bearer xyz\n".as_slice(),
        b"endpoint = postgres://u:p@host/db\n".as_slice(),
        b"-----BEGIN PRIVATE KEY-----\nabc\n-----END PRIVATE KEY-----\n".as_slice(),
    ] {
        let error = prepare(&store, SESSION, "src/lib.rs", Some(content), 1_000).unwrap_err();
        assert_eq!(
            error,
            CheckpointStoreError::Screened(ScreenRefusal::SecretContent)
        );
        assert!(
            !state.join("sessions").exists(),
            "secret content refused before any write"
        );
    }
    let _ = fs::remove_dir_all(state);
}
#[test]
fn screening_refuses_non_utf8_private_key_bytes_but_admits_plain_binary() {
    let (state, store) = ck_store("screen-nonutf8");
    let mut pem = vec![0xffu8, 0xfe];
    pem.extend_from_slice(b"garbage \x80\x81 ");
    pem.extend_from_slice(b"-----Begin Private Key-----\n\xff");
    let error = prepare(&store, SESSION, "a.bin", Some(&pem), 1_000).unwrap_err();
    assert_eq!(
        error,
        CheckpointStoreError::Screened(ScreenRefusal::SecretContent)
    );
    assert!(!state.join("sessions").exists());

    let binary = [0xffu8, 0xfe, 0x00, 0x01];
    prepare(&store, SESSION, "b.bin", Some(&binary), 1_000).unwrap();
    let _ = fs::remove_dir_all(state);
}
#[test]
fn screening_refuses_database_export_paths_and_magics() {
    let (state, store) = ck_store("screen-exports");
    for path in [
        "data/out.parquet",
        "x.duckdb",
        "y.sqlite3",
        "saya-chart-0123456789abcdef.html",
    ] {
        let error = prepare(&store, SESSION, path, Some(b"x"), 1_000).unwrap_err();
        assert_eq!(
            error,
            CheckpointStoreError::Screened(ScreenRefusal::DatabaseExport),
            "path {path:?}"
        );
        assert!(
            !state.join("sessions").exists(),
            "path {path:?} refused before any write"
        );
    }
    let parquet = b"PAR1col".to_vec();
    let sqlite = b"SQLite format 3\0page".to_vec();
    let mut duck = vec![0x00u8; 8];
    duck.extend_from_slice(b"DUCK");
    duck.extend_from_slice(b"v1.0");
    for (path, magic) in [
        ("report.bin", parquet.as_slice()),
        ("report.bin", sqlite.as_slice()),
        ("report.bin", duck.as_slice()),
    ] {
        let error = prepare(&store, SESSION, path, Some(magic), 1_000).unwrap_err();
        assert_eq!(
            error,
            CheckpointStoreError::Screened(ScreenRefusal::DatabaseExport)
        );
        assert!(!state.join("sessions").exists());
    }
    for path in ["report.html", "chart.html"] {
        prepare(&store, SESSION, path, Some(b"<html></html>"), 1_000)
            .unwrap_or_else(|error| panic!("path {path:?} must be admitted: {error:?}"));
    }
    let _ = fs::remove_dir_all(state);
}
#[test]
fn screening_applies_to_creates_without_a_before_image() {
    let (state, store) = ck_store("screen-create");
    let error = prepare(&store, SESSION, ".env", None, 1_000).unwrap_err();
    assert_eq!(
        error,
        CheckpointStoreError::Screened(ScreenRefusal::CredentialPath)
    );
    let export = prepare(&store, SESSION, "x.sqlite", None, 1_000).unwrap_err();
    assert_eq!(
        export,
        CheckpointStoreError::Screened(ScreenRefusal::DatabaseExport)
    );
    assert!(
        !state.join("sessions").exists(),
        "refused creates write nothing"
    );
    prepare(&store, SESSION, "new/file.txt", None, 1_000).unwrap();
    let _ = fs::remove_dir_all(state);
}
#[test]
fn screened_refusals_never_echo_the_path_or_the_content() {
    let (state, store) = ck_store("screen-echo");
    let path_error = prepare(
        &store,
        SESSION,
        "keys/SENTINELPATH42.pem",
        Some(b"x"),
        1_000,
    )
    .unwrap_err();
    let content_error = prepare(
        &store,
        SESSION,
        "a.txt",
        Some(b"password=SENTINELCONTENT7\n"),
        1_000,
    )
    .unwrap_err();
    let export_error =
        prepare(&store, SESSION, "SENTINELPATH42.parquet", Some(b"x"), 1_000).unwrap_err();
    for error in [path_error, content_error, export_error] {
        let message = error.to_string();
        assert!(!message.contains("SENTINELPATH42"));
        assert!(!message.contains("SENTINELCONTENT7"));
    }
    let _ = fs::remove_dir_all(state);
}
