//! The scratch semantics battery (ADR 0003 §5, M4-1): pins what the bundled
//! DuckDB 1.10505.0 actually does under the narrowed scratch configuration
//! (connector hardening from `duckdb/client.rs:43-52` with external access on)
//! before `scratch_sql` exists. Each test name states a claim; where observed
//! reality differs, the name was amended and the surprise is recorded inline.

use std::{fs, path::Path, path::PathBuf, sync::mpsc, time::Duration};

use duckdb::{AccessMode, Connection, params};

/// The scratch configuration: the connector's hardening verbatim, with
/// `enable_external_access(true)` per ADR 0003 decision 4.
fn scratch_config() -> duckdb::Config {
    duckdb::Config::default()
        .access_mode(AccessMode::ReadWrite)
        .and_then(|item| item.enable_external_access(true))
        .and_then(|item| item.enable_autoload_extension(false))
        .and_then(|item| item.with("allow_community_extensions", "false"))
        .and_then(|item| item.with("allow_persistent_secrets", "false"))
        .and_then(|item| item.with("lock_configuration", "true"))
        .expect("scratch security configuration must build")
}

fn open(path: &Path) -> Connection {
    Connection::open_with_flags(path, scratch_config()).expect("scratch open must succeed")
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("saya-scratch-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn read_write_open_creates_a_missing_database_file() {
    let dir = TempDir::new("create-file");
    let path = dir.path.join("scratch.duckdb");
    assert!(!path.exists());
    let _conn = open(&path);
    assert!(path.exists(), "ReadWrite open must create the missing file");
}

#[test]
fn read_write_open_does_not_create_missing_parent_directories() {
    let dir = TempDir::new("missing-parents");
    let path = dir.path.join("missing-one/missing-two/scratch.duckdb");
    assert!(Connection::open_with_flags(&path, scratch_config()).is_err());
    assert!(
        !dir.path.join("missing-one").exists(),
        "the engine must not create parent directories"
    );
}

#[cfg(unix)]
/// **Reality contradicted the design.** DuckDB creates the database file with
/// mode 0644 — group and world readable. The run directory is 0700
/// (`run_dir.rs`), so the file is unreachable in practice today, but the
/// scratch file carries no protection of its own: anything that ever widens
/// the run directory exposes every staged row. Pinned as observed, not as
/// hoped; the caller must chmod after create if it wants defence in depth.
#[test]
#[cfg(unix)]
fn a_created_database_file_is_created_group_and_world_readable() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new("mode");
    let path = dir.path.join("scratch.duckdb");
    let _conn = open(&path);
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o644,
        "observed DuckDB's own creation mode; if this changes, revisit the ADR"
    );
}

#[test]
fn ddl_and_dml_round_trip_on_a_scratch_file() {
    let dir = TempDir::new("round-trip");
    let conn = open(&dir.path.join("scratch.duckdb"));
    conn.execute_batch(
        "CREATE TABLE t(a INTEGER, b VARCHAR);
         INSERT INTO t VALUES (1, 'alpha'), (2, 'beta');",
    )
    .unwrap();
    let mut stmt = conn.prepare("SELECT a, b FROM t ORDER BY a").unwrap();
    let rows: Vec<(i32, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![(1, "alpha".to_string()), (2, "beta".to_string())]
    );
}

#[test]
fn configuration_stays_locked_after_open() {
    let dir = TempDir::new("locked");
    let conn = open(&dir.path.join("scratch.duckdb"));
    let tighten = conn.execute_batch("SET enable_external_access = false");
    let widen = conn.execute_batch("SET enable_external_access = true");
    assert!(
        tighten.is_err(),
        "SET to false must also be refused while locked"
    );
    assert!(widen.is_err(), "SET to true must be refused while locked");
}

/// **Reality contradicted the design, and this one is a security finding.**
/// ADR 0003 decision 4 assumed that `enable_autoload_extension(false)` plus
/// community-extension denial plus `lock_configuration` would keep extensions
/// out while `enable_external_access(true)` let corpus files load. They do
/// not: `httpfs` is a *core* extension, so the community denial does not
/// touch it, autoload governs implicit loading rather than an explicit
/// statement, and `lock_configuration` locks settings rather than `INSTALL`.
/// Both statements succeed, and `INSTALL` reaches DuckDB's extension
/// repository over the network to do it.
#[test]
fn installing_a_core_extension_is_permitted_under_external_access() {
    let dir = TempDir::new("extension");
    let conn = open(&dir.path.join("scratch.duckdb"));
    assert!(
        conn.execute_batch("INSTALL httpfs").is_ok(),
        "observed: INSTALL is not blocked by this configuration"
    );
    assert!(
        conn.execute_batch("LOAD httpfs").is_ok(),
        "observed: LOAD is not blocked by this configuration"
    );
}

/// The consequence, pinned so it cannot regress unnoticed: once `httpfs` is
/// loaded the connection makes outbound TCP connections, which is an egress
/// path that never crosses the fetch policy (M3-1). Served from 127.0.0.1 so
/// the proof costs no external packet. The discriminator is the *error kind*:
/// with external access on the read fails at the network layer, and with it
/// off the same statement fails at DuckDB's permission layer — the second
/// never leaves the process.
#[test]
fn a_loaded_httpfs_reaches_the_network_which_the_fetch_policy_never_sees() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let _ = listener.accept();
    });
    let dir = TempDir::new("egress");
    let conn = open(&dir.path.join("scratch.duckdb"));
    let _ = conn.execute_batch("INSTALL httpfs");
    let _ = conn.execute_batch("LOAD httpfs");
    let error = conn
        .execute_batch(&format!("SELECT * FROM read_csv('http://{addr}/x.csv')"))
        .expect_err("the toy listener never answers, so the read must fail")
        .to_string();
    assert!(
        !error.contains("Permission Error"),
        "the statement reached the network instead of being refused: {error}"
    );
}

/// The fallback ADR 0003 recorded, measured: with external access off, every
/// route out is refused at the permission layer — `INSTALL`, `LOAD`, an http
/// read *and a local file read*. The flag is all-or-nothing; there is no
/// "local files yes, network no" setting. So the fallback is not a milder
/// version of decision 4, it is the whole of it: no `read_csv` inside the
/// engine at all, and corpus loading happens outside.
#[test]
fn external_access_off_refuses_local_reads_too_so_the_fallback_is_all_or_nothing() {
    let dir = TempDir::new("fallback");
    let csv = dir.path.join("d.csv");
    std::fs::write(&csv, "a,b\n1,x\n").unwrap();
    let conn = Connection::open_with_flags(
        dir.path.join("scratch.duckdb"),
        duckdb::Config::default()
            .access_mode(AccessMode::ReadWrite)
            .and_then(|item| item.enable_external_access(false))
            .and_then(|item| item.enable_autoload_extension(false))
            .and_then(|item| item.with("allow_community_extensions", "false"))
            .and_then(|item| item.with("allow_persistent_secrets", "false"))
            .and_then(|item| item.with("lock_configuration", "true"))
            .expect("fallback configuration must build"),
    )
    .expect("fallback open must succeed");
    for statement in [
        "INSTALL httpfs".to_string(),
        "LOAD httpfs".to_string(),
        format!("SELECT * FROM read_csv('{}')", csv.display()),
    ] {
        let error = conn
            .execute_batch(&statement)
            .expect_err("every file route must be refused")
            .to_string();
        assert!(
            error.contains("Permission Error"),
            "{statement} was not refused at the permission layer: {error}"
        );
    }
    // DDL and DML still work: the scratch join/score capability survives.
    conn.execute_batch("CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (1)")
        .expect("writes to the scratch file itself stay available");
}

#[test]
fn read_csv_reads_a_local_file_under_external_access() {
    let dir = TempDir::new("read-csv");
    let csv = dir.path.join("corpus.csv");
    fs::write(&csv, "id,name\n1,alpha\n2,beta\n").unwrap();
    let conn = open(&dir.path.join("scratch.duckdb"));
    let mut stmt = conn
        .prepare(&format!(
            "SELECT id, name FROM read_csv('{}', header = true) ORDER BY id",
            csv.display()
        ))
        .unwrap();
    let rows: Vec<(i64, String)> = stmt
        .query_map(params![], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![(1, "alpha".to_string()), (2, "beta".to_string())]
    );
}

#[test]
fn an_https_url_is_not_readable_without_httpfs() {
    let dir = TempDir::new("https");
    let conn = open(&dir.path.join("scratch.duckdb"));
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        // `example.invalid` is an RFC 2606 reserved TLD: no real DNS lookup can
        // succeed, so a hang here is an engine regression, not a network flake.
        let refused = conn
            .execute_batch("SELECT * FROM read_csv('https://example.invalid/x.csv')")
            .is_err();
        let _ = tx.send(refused);
    });
    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(refused) => assert!(refused, "https read_csv must error without httpfs"),
        Err(_) => panic!("https read_csv did not return within 10s; must never wedge CI"),
    }
}
