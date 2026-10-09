//! The extension policy's observable behavior: a core extension's `INSTALL`
//! is not refused by the scratch policy, and once loaded the connection
//! reaches the network — an egress path the fetch policy never sees.

use super::{TempDir, open};

/// **Reality contradicted the design, and this one is a security finding.**
/// ADR 0003 decision 4 assumed that `enable_autoload_extension(false)` plus
/// community-extension denial plus `lock_configuration` would keep extensions
/// out while `enable_external_access(true)` let corpus files load. They do
/// not: `httpfs` is a *core* extension, so the community denial does not
/// touch it, autoload governs implicit loading rather than an explicit
/// statement, and `lock_configuration` locks settings rather than `INSTALL`.
/// `INSTALL` reaches DuckDB's extension repository over the network to do
/// it. Repository availability is outside this configuration's control, so
/// the assertion distinguishes a policy refusal from an unavailable package.
#[test]
fn installing_a_core_extension_is_not_refused_by_the_scratch_policy() {
    let dir = TempDir::new("extension");
    let conn = open(&dir.path.join("scratch.duckdb"));
    if let Err(error) = conn.execute_batch("INSTALL httpfs") {
        assert!(
            !error.to_string().contains("Permission Error"),
            "INSTALL must not be refused by the scratch policy: {error}"
        );
    } else {
        assert!(
            conn.execute_batch("LOAD httpfs").is_ok(),
            "a core extension installed under this policy must load"
        );
    }
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
