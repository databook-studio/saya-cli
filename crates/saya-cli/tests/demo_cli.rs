//! Integration tests for `saya demo` (S13): the headless path builds a
//! deterministic synthetic SQLite database in an isolated demo directory and
//! prints how to open it; the generated connections profile then runs through
//! the normal CLI read path (SELECT ok, DELETE refused), and reuse/reset
//! behave as pinned by Milestone A D11.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::SystemTime,
};

fn isolated_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("saya-cli-demo-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn db_path(root: &Path) -> PathBuf {
    root.join("demo").join("demo.sqlite3")
}

fn conn_path(root: &Path) -> PathBuf {
    root.join("demo").join("connections.toml")
}

fn demo_command(root: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_saya"));
    cmd.env("SAYA_DEMO_DIR", root.join("demo"))
        .env("SAYA_CONFIG_HOME", root.join("user-config"))
        .env("SAYA_STATE_DB", root.join("state.sqlite3"))
        .env("HOME", root);
    cmd
}

fn query_command(root: &Path, sql: &str) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_saya"));
    cmd.args(["--non-interactive", "--format", "json", "--connections"])
        .arg(conn_path(root))
        .args(["--profile", "demo", "query", "--sql", sql])
        .env("SAYA_CONFIG_HOME", root.join("user-config"))
        .env("SAYA_STATE_DB", root.join("state.sqlite3"))
        .env("HOME", root);
    cmd
}

fn modified(path: &Path) -> SystemTime {
    fs::metadata(path).unwrap().modified().unwrap()
}

fn stderr_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn demo_opens_read_only_and_needs_no_database_server() {
    let root = isolated_root("main");
    let db = db_path(&root);
    let conn = conn_path(&root);

    // a) Headless: prints the paths, the launch command, and example SQL;
    //    exits 0; writes the database and connections files. No TTY launch.
    let out = demo_command(&root)
        .args(["demo", "--non-interactive", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "demo must succeed headless, stderr: {}",
        stderr_of(&out)
    );
    let stdout = stdout_of(&out);
    assert!(
        stdout.contains(db.to_str().unwrap()),
        "stdout names the db path: {stdout}"
    );
    assert!(
        stdout.contains(conn.to_str().unwrap()),
        "stdout names the connections path: {stdout}"
    );
    assert!(
        stdout.contains("--profile demo"),
        "stdout names the launch command: {stdout}"
    );
    assert!(
        stdout.contains("SELECT count(*) FROM customers"),
        "stdout shows example SQL: {stdout}"
    );
    assert!(db.exists(), "the demo database file exists");
    assert!(conn.exists(), "the connections file exists");

    // b) The generated profile answers SELECT through the normal CLI path —
    //    no network, no provider, no database server.
    let select = query_command(&root, "SELECT count(*) FROM customers")
        .output()
        .unwrap();
    assert_eq!(
        select.status.code(),
        Some(0),
        "select must succeed, stderr: {}",
        stderr_of(&select)
    );
    assert!(
        stdout_of(&select).contains("\"event\":\"query_result\""),
        "select renders a query result: {}",
        stdout_of(&select)
    );

    // c) Writes through the same profile are refused (safety exit 4): the
    //    demo database is opened read-only.
    let delete = query_command(&root, "DELETE FROM customers")
        .output()
        .unwrap();
    assert_eq!(
        delete.status.code(),
        Some(4),
        "DELETE must be refused, stderr: {}",
        stderr_of(&delete)
    );
    assert!(
        stderr_of(&delete).contains("\"event\":\"error\""),
        "refusal renders an error event: {}",
        stderr_of(&delete)
    );
    assert_eq!(
        fs::read_to_string(&conn)
            .unwrap()
            .matches("[profiles.demo]")
            .count(),
        1,
        "the connections file declares exactly one demo profile"
    );

    // d) A second run reuses the existing fixture: the db file is untouched.
    let before = modified(&db);
    let reuse = demo_command(&root)
        .args(["demo", "--non-interactive"])
        .output()
        .unwrap();
    assert_eq!(
        reuse.status.code(),
        Some(0),
        "reuse run must succeed, stderr: {}",
        stderr_of(&reuse)
    );
    assert_eq!(
        modified(&db),
        before,
        "reuse must not rewrite the database file"
    );

    // e) --reset rebuilds the fixture in place.
    let reset = demo_command(&root)
        .args(["demo", "--non-interactive", "--reset"])
        .output()
        .unwrap();
    assert_eq!(
        reset.status.code(),
        Some(0),
        "reset run must succeed, stderr: {}",
        stderr_of(&reset)
    );
    assert_ne!(
        modified(&db),
        before,
        "--reset must rebuild the database file"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn demo_defaults_to_the_state_db_parent_directory() {
    let root = isolated_root("default-dir");
    let out = Command::new(env!("CARGO_BIN_EXE_saya"))
        .args(["demo", "--non-interactive", "--format", "json"])
        .env("SAYA_CONFIG_HOME", root.join("user-config"))
        .env("SAYA_STATE_DB", root.join("state.sqlite3"))
        .env("HOME", &root)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "demo must succeed headless, stderr: {}",
        stderr_of(&out)
    );
    assert!(db_path(&root).exists(), "db lands beside the state db");
    assert!(
        conn_path(&root).exists(),
        "connections land in the demo dir"
    );
    let _ = fs::remove_dir_all(&root);
}
