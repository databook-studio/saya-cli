use saya_cli::{resolve_session_dir, resolve_state_db_path};

#[test]
fn session_path_resolution_is_platform_aware_and_injectable() {
    assert_eq!(
        resolve_session_dir(
            Some("/override"),
            Some("/xdg"),
            Some("/appdata"),
            Some("/home")
        ),
        std::path::PathBuf::from("/override")
    );
    assert_eq!(
        resolve_session_dir(None, Some("/xdg"), Some("/appdata"), Some("/home")),
        std::path::PathBuf::from("/xdg/saya/sessions")
    );
    assert_eq!(
        resolve_session_dir(None, None, Some("/appdata"), Some("/home")),
        std::path::PathBuf::from("/appdata/saya/sessions")
    );
    assert_eq!(
        resolve_session_dir(None, None, None, Some("/home")),
        std::path::PathBuf::from("/home/.local/share/saya/sessions")
    );
}

#[test]
fn state_path_precedence_is_platform_aware_and_injectable() {
    assert_eq!(
        resolve_state_db_path(
            Some("/override.sqlite3"),
            Some("/xdg"),
            Some("/appdata"),
            Some("/home")
        ),
        std::path::PathBuf::from("/override.sqlite3")
    );
    assert_eq!(
        resolve_state_db_path(None, Some("/xdg"), Some("/appdata"), Some("/home")),
        std::path::PathBuf::from("/xdg/saya/state.sqlite3")
    );
    assert_eq!(
        resolve_state_db_path(None, None, Some("/appdata"), Some("/home")),
        std::path::PathBuf::from("/appdata/saya/state.sqlite3")
    );
}

#[cfg(unix)]
#[test]
fn relative_and_existing_parent_state_paths_do_not_change_parent_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("saya-cli-relative-state-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
    let database = root.join("data.duckdb");
    duckdb::Connection::open(&database)
        .unwrap()
        .execute_batch("CREATE TABLE events (id INTEGER);")
        .unwrap();
    let connections = root.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.analytics]\ntype = 'duckdb'\npath = '{}'\nread_only = true\n",
            database.display()
        ),
    )
    .unwrap();
    let state_paths = [
        std::ffi::OsString::from("relative-state.sqlite3"),
        root.join("shared-state.sqlite3").into_os_string(),
    ];
    for state_path in state_paths {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
            .current_dir(&root)
            .args([
                "--non-interactive",
                "--connections",
                connections.to_str().unwrap(),
                "connection",
                "schema",
                "analytics",
            ])
            .env("SAYA_STATE_DB", state_path)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0));
    }
    assert_eq!(
        std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert!(root.join("relative-state.sqlite3").exists());
    assert!(root.join("shared-state.sqlite3").exists());
    let _ = std::fs::remove_dir_all(root);
}
