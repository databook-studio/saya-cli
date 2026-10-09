//! The locked configuration stays locked: after open, both directions of a
//! `SET` on the hardened settings are refused.

use super::{TempDir, open};

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
