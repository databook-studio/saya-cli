//! What the external-access flag itself buys: with it on, local corpus
//! files read but a remote URL needs `httpfs`; with it off, every route out
//! is refused at the permission layer and the fallback is all-or-nothing.

use std::fs;
use std::sync::mpsc;
use std::time::Duration;

use duckdb::{AccessMode, Connection, params};

use super::{TempDir, open};

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
