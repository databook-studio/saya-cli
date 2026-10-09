//! Cross-backend schema fixtures: every shipped fixture parses into a
//! `SchemaTree`, and the same `orders` table fingerprints differently per
//! backend.

use super::*;

fn load_fixture(name: &str) -> SchemaTree {
    let json = match name {
        "postgres.json" => include_str!("../fixtures/schemas/postgres.json"),
        "mysql.json" => include_str!("../fixtures/schemas/mysql.json"),
        "sqlite.json" => include_str!("../fixtures/schemas/sqlite.json"),
        "duckdb.json" => include_str!("../fixtures/schemas/duckdb.json"),
        "snowflake.json" => include_str!("../fixtures/schemas/snowflake.json"),
        _ => panic!("unknown fixture: {name}"),
    };
    serde_json::from_str(json).unwrap()
}

fn find_orders(tree: &SchemaTree) -> &Table {
    tree.databases
        .iter()
        .flat_map(|db| db.schemas.iter())
        .flat_map(|s| s.tables.iter())
        .find(|t| t.name.eq_ignore_ascii_case("orders"))
        .expect("orders table not found")
}

#[test]
fn test_8_all_fixtures_parse_and_orders_has_6_columns() {
    for name in &[
        "postgres.json",
        "mysql.json",
        "sqlite.json",
        "duckdb.json",
        "snowflake.json",
    ] {
        let tree = load_fixture(name);
        let orders = find_orders(&tree);
        assert_eq!(orders.columns.len(), 6, "failed for {name}");
    }
}

#[test]
fn test_8_orders_fingerprint_differs_across_backends() {
    let mut fingerprints = HashSet::new();
    for name in &[
        "postgres.json",
        "mysql.json",
        "sqlite.json",
        "duckdb.json",
        "snowflake.json",
    ] {
        let tree = load_fixture(name);
        let orders = find_orders(&tree);
        let fp = SchemaFingerprint::of_table(DatabaseObjectKind::Table, orders);
        assert!(fingerprints.insert(fp), "duplicate fingerprint for {name}");
    }
    assert_eq!(fingerprints.len(), 5);
}
