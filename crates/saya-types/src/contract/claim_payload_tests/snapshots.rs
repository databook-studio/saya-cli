use super::*;

fn table_with(cols: &[(&str, &str, bool)]) -> crate::Table {
    crate::Table {
        name: "t".into(),
        columns: cols
            .iter()
            .map(|(name, ty, nullable)| crate::Column {
                name: (*name).into(),
                data_type: (*ty).into(),
                nullable: *nullable,
            })
            .collect(),
        primary_key: vec![],
        foreign_keys: vec![],
    }
}

#[test]
fn snapshots_resolve_type_and_nullability_from_the_live_table() {
    let table = table_with(&[("user_id", "bigint", false), ("amount", "numeric", true)]);
    let payload = ClaimPayload::column_description("user_id", "the user id").unwrap();
    assert_eq!(
        payload.referenced_column_snapshots(&table),
        vec![ReferencedColumn {
            name: "user_id".into(),
            data_type: "bigint".into(),
            nullable: false,
        }]
    );
}

#[test]
fn snapshot_skips_a_referenced_column_absent_from_the_table() {
    // A claim cannot snapshot what does not exist; the caller decides what
    // that means. The helper returns only the columns it could resolve.
    let table = table_with(&[("id", "bigint", false)]);
    let payload = ClaimPayload::column_description("missing", "gone").unwrap();
    assert!(payload.referenced_column_snapshots(&table).is_empty());
}

#[test]
fn snapshots_match_case_insensitively_like_fingerprinting_does() {
    // Schema discovery and fingerprint comparison are case-insensitive on
    // object and column names; a snapshot must resolve the same way or a
    // retyped column would read as "absent" instead of "changed".
    let table = table_with(&[("User_Id", "bigint", false)]);
    let payload = ClaimPayload::column_description("user_id", "the id").unwrap();
    assert_eq!(
        payload.referenced_column_snapshots(&table),
        vec![ReferencedColumn {
            name: "user_id".into(),
            data_type: "bigint".into(),
            nullable: false,
        }]
    );
}

#[test]
fn relationship_snapshots_all_local_columns() {
    let table = table_with(&[
        ("local_a", "int", false),
        ("local_b", "text", true),
        ("unrelated", "int", false),
    ]);
    let target = make_target();
    let payload = ClaimPayload::relationship(
        target,
        vec!["local_a".into(), "local_b".into()],
        vec!["x".into(), "y".into()],
        Cardinality::OneToMany,
    )
    .unwrap();
    assert_eq!(
        payload.referenced_column_snapshots(&table),
        vec![
            ReferencedColumn {
                name: "local_a".into(),
                data_type: "int".into(),
                nullable: false,
            },
            ReferencedColumn {
                name: "local_b".into(),
                data_type: "text".into(),
                nullable: true,
            },
        ]
    );
}

#[test]
fn table_level_claims_snapshot_nothing() {
    let table = table_with(&[("id", "bigint", false)]);
    assert!(
        ClaimPayload::table_description("desc")
            .unwrap()
            .referenced_column_snapshots(&table)
            .is_empty()
    );
    assert!(
        ClaimPayload::table_alias("a")
            .unwrap()
            .referenced_column_snapshots(&table)
            .is_empty()
    );
}

#[test]
fn name_only_snapshots_record_names_without_claiming_a_type() {
    // A no-schema caller records the column names a drift rule needs, but
    // with an empty type so the reconciler treats them as unknown.
    let payload = ClaimPayload::column_description("user_id", "the id").unwrap();
    assert_eq!(
        payload.referenced_column_name_snapshots(),
        vec![ReferencedColumn {
            name: "user_id".into(),
            data_type: String::new(),
            nullable: false,
        }]
    );
    assert!(
        ClaimPayload::table_description("desc")
            .unwrap()
            .referenced_column_name_snapshots()
            .is_empty()
    );
}

#[test]
fn referenced_column_serde_round_trips() {
    let col = ReferencedColumn {
        name: "amount".into(),
        data_type: "numeric".into(),
        nullable: true,
    };
    let json = serde_json::to_string(&col).unwrap();
    let back: ReferencedColumn = serde_json::from_str(&json).unwrap();
    assert_eq!(col, back);
}
