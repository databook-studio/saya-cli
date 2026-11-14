//! `SchemaFingerprint` sensitivity: the digest is stable for an unchanged
//! table and changes on rename, retypes, nullability, reordering, added or
//! removed columns, object kind, and length-prefix collisions.

use super::*;

fn make_table(columns: Vec<(&str, &str, bool)>) -> Table {
    Table {
        name: "t".into(),
        columns: columns
            .into_iter()
            .map(|(name, data_type, nullable)| saya_types::Column {
                name: name.into(),
                data_type: data_type.into(),
                nullable,
            })
            .collect(),
        primary_key: vec![],
        foreign_keys: vec![],
    }
}

#[test]
fn test_5_fingerprint_is_stable() {
    let table = valid_table();
    let fp1 = SchemaFingerprint::of_table(DatabaseObjectKind::Table, &table);
    let fp2 = SchemaFingerprint::of_table(DatabaseObjectKind::Table, &table);
    assert_eq!(fp1, fp2);
}

#[test]
fn test_6_fingerprint_changes_on_column_renamed() {
    let a = make_table(vec![("id", "int", false)]);
    let b = make_table(vec![("uid", "int", false)]);
    assert_ne!(
        SchemaFingerprint::of_table(DatabaseObjectKind::Table, &a),
        SchemaFingerprint::of_table(DatabaseObjectKind::Table, &b)
    );
}

#[test]
fn test_6_fingerprint_changes_on_retyped() {
    let a = make_table(vec![("id", "int", false)]);
    let b = make_table(vec![("id", "bigint", false)]);
    assert_ne!(
        SchemaFingerprint::of_table(DatabaseObjectKind::Table, &a),
        SchemaFingerprint::of_table(DatabaseObjectKind::Table, &b)
    );
}

#[test]
fn test_6_fingerprint_changes_on_nullable() {
    let a = make_table(vec![("id", "int", false)]);
    let b = make_table(vec![("id", "int", true)]);
    assert_ne!(
        SchemaFingerprint::of_table(DatabaseObjectKind::Table, &a),
        SchemaFingerprint::of_table(DatabaseObjectKind::Table, &b)
    );
}

#[test]
fn test_6_fingerprint_changes_on_reordered() {
    let a = make_table(vec![("a", "int", false), ("b", "text", true)]);
    let b = make_table(vec![("b", "text", true), ("a", "int", false)]);
    assert_ne!(
        SchemaFingerprint::of_table(DatabaseObjectKind::Table, &a),
        SchemaFingerprint::of_table(DatabaseObjectKind::Table, &b)
    );
}

#[test]
fn test_6_fingerprint_changes_on_added() {
    let a = make_table(vec![("id", "int", false)]);
    let b = make_table(vec![("id", "int", false), ("name", "text", true)]);
    assert_ne!(
        SchemaFingerprint::of_table(DatabaseObjectKind::Table, &a),
        SchemaFingerprint::of_table(DatabaseObjectKind::Table, &b)
    );
}

#[test]
fn test_6_fingerprint_changes_on_removed() {
    let a = make_table(vec![("id", "int", false), ("name", "text", true)]);
    let b = make_table(vec![("id", "int", false)]);
    assert_ne!(
        SchemaFingerprint::of_table(DatabaseObjectKind::Table, &a),
        SchemaFingerprint::of_table(DatabaseObjectKind::Table, &b)
    );
}

#[test]
fn test_6_fingerprint_differs_table_vs_view() {
    let table = valid_table();
    assert_ne!(
        SchemaFingerprint::of_table(DatabaseObjectKind::Table, &table),
        SchemaFingerprint::of_table(DatabaseObjectKind::View, &table)
    );
}

#[test]
fn test_7_length_prefix_collision() {
    let a = make_table(vec![("a", "bc", false)]);
    let b = make_table(vec![("ab", "c", false)]);
    assert_ne!(
        SchemaFingerprint::of_table(DatabaseObjectKind::Table, &a),
        SchemaFingerprint::of_table(DatabaseObjectKind::Table, &b)
    );
}
