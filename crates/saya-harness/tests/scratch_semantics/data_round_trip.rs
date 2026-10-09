//! The staged data survives a scratch session: DDL and DML round-trip on
//! the scratch file and read back in order.

use super::{TempDir, open};

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
