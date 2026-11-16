use super::*;
use crate::Column;

fn make_table(columns: Vec<(&str, &str, bool)>) -> Table {
    Table {
        name: "t".into(),
        columns: columns
            .into_iter()
            .map(|(name, data_type, nullable)| Column {
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
fn fingerprint_is_stable() {
    let table = make_table(vec![("id", "bigint", false)]);
    let kind = DatabaseObjectKind::Table;
    let fp1 = SchemaFingerprint::of_table(kind, &table);
    let fp2 = SchemaFingerprint::of_table(kind, &table);
    assert_eq!(fp1, fp2);
}

#[test]
fn changes_when_column_renamed() {
    let table_a = make_table(vec![("id", "bigint", false)]);
    let table_b = make_table(vec![("uid", "bigint", false)]);
    let kind = DatabaseObjectKind::Table;
    assert_ne!(
        SchemaFingerprint::of_table(kind, &table_a),
        SchemaFingerprint::of_table(kind, &table_b)
    );
}

#[test]
fn changes_when_column_retyped() {
    let table_a = make_table(vec![("id", "bigint", false)]);
    let table_b = make_table(vec![("id", "int", false)]);
    let kind = DatabaseObjectKind::Table;
    assert_ne!(
        SchemaFingerprint::of_table(kind, &table_a),
        SchemaFingerprint::of_table(kind, &table_b)
    );
}

#[test]
fn changes_when_column_made_nullable() {
    let table_a = make_table(vec![("id", "bigint", false)]);
    let table_b = make_table(vec![("id", "bigint", true)]);
    let kind = DatabaseObjectKind::Table;
    assert_ne!(
        SchemaFingerprint::of_table(kind, &table_a),
        SchemaFingerprint::of_table(kind, &table_b)
    );
}

#[test]
fn changes_when_column_reordered() {
    let table_a = make_table(vec![("a", "int", false), ("b", "text", true)]);
    let table_b = make_table(vec![("b", "text", true), ("a", "int", false)]);
    let kind = DatabaseObjectKind::Table;
    assert_ne!(
        SchemaFingerprint::of_table(kind, &table_a),
        SchemaFingerprint::of_table(kind, &table_b)
    );
}

#[test]
fn changes_when_column_added() {
    let table_a = make_table(vec![("id", "bigint", false)]);
    let table_b = make_table(vec![("id", "bigint", false), ("name", "text", true)]);
    let kind = DatabaseObjectKind::Table;
    assert_ne!(
        SchemaFingerprint::of_table(kind, &table_a),
        SchemaFingerprint::of_table(kind, &table_b)
    );
}

#[test]
fn changes_when_column_removed() {
    let table_a = make_table(vec![("id", "bigint", false), ("name", "text", true)]);
    let table_b = make_table(vec![("id", "bigint", false)]);
    let kind = DatabaseObjectKind::Table;
    assert_ne!(
        SchemaFingerprint::of_table(kind, &table_a),
        SchemaFingerprint::of_table(kind, &table_b)
    );
}

#[test]
fn differs_between_table_and_view() {
    let table = make_table(vec![("id", "bigint", false)]);
    assert_ne!(
        SchemaFingerprint::of_table(DatabaseObjectKind::Table, &table),
        SchemaFingerprint::of_table(DatabaseObjectKind::View, &table)
    );
}

#[test]
fn length_prefix_collision() {
    let table_a = make_table(vec![("a", "bc", false)]);
    let table_b = make_table(vec![("ab", "c", false)]);
    assert_ne!(
        SchemaFingerprint::of_table(DatabaseObjectKind::Table, &table_a),
        SchemaFingerprint::of_table(DatabaseObjectKind::Table, &table_b)
    );
}

#[test]
fn version_is_correct() {
    let table = make_table(vec![("id", "bigint", false)]);
    let fp = SchemaFingerprint::of_table(DatabaseObjectKind::Table, &table);
    assert_eq!(fp.version(), FINGERPRINT_VERSION);
}

#[test]
fn display_and_as_str() {
    let table = make_table(vec![("id", "bigint", false)]);
    let fp = SchemaFingerprint::of_table(DatabaseObjectKind::Table, &table);
    assert_eq!(fp.as_str(), &format!("{fp}"));
}
