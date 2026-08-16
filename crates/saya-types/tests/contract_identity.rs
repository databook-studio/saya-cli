use std::collections::HashSet;

use saya_types::{
    Cardinality, ClaimOrigin, ClaimPayload, ClaimStatus, ContractError, DatabaseObjectKind,
    DatabaseObjectRef, FINGERPRINT_VERSION, MAX_REFERENCED_COLUMNS, ProfileIdentity,
    SchemaFingerprint, SchemaTree, Table,
};

fn valid_hex() -> String {
    "a".repeat(64)
}

fn valid_profile() -> ProfileIdentity {
    ProfileIdentity::parse(&format!("p-{}", valid_hex())).unwrap()
}

fn valid_table() -> Table {
    Table {
        name: "orders".into(),
        columns: vec![saya_types::Column {
            name: "id".into(),
            data_type: "bigint".into(),
            nullable: false,
        }],
    }
}

#[test]
fn test_1_profile_identity_accepts_valid() {
    let hex = valid_hex();
    let input = format!("p-{hex}");
    let id = ProfileIdentity::parse(&input).unwrap();
    assert_eq!(id.as_str(), &input);
}

#[test]
fn test_1_profile_identity_rejects_too_short() {
    assert!(ProfileIdentity::parse("p-abc").is_err());
}

#[test]
fn test_1_profile_identity_rejects_too_long() {
    let input = format!("p-{}", "a".repeat(65));
    assert!(ProfileIdentity::parse(&input).is_err());
}

#[test]
fn test_1_profile_identity_rejects_missing_prefix() {
    assert!(ProfileIdentity::parse(&valid_hex()).is_err());
}

#[test]
fn test_1_profile_identity_rejects_uppercase_hex() {
    let input = format!("p-{}", "A".repeat(64));
    assert!(ProfileIdentity::parse(&input).is_err());
}

#[test]
fn test_1_profile_identity_rejects_non_hex() {
    let input = format!("p-{}", "g".repeat(64));
    assert!(ProfileIdentity::parse(&input).is_err());
}

#[test]
fn test_2_profile_identity_serde_round_trip() {
    let id = valid_profile();
    let json = serde_json::to_string(&id).unwrap();
    let deserialized: ProfileIdentity = serde_json::from_str(&json).unwrap();
    assert_eq!(id, deserialized);
}

#[test]
fn test_2_profile_identity_serde_rejects_invalid() {
    let result: Result<ProfileIdentity, _> = serde_json::from_str(r#""p-INVALID""#);
    assert!(result.is_err());
}

#[test]
fn test_3_object_ref_rejects_empty_name() {
    let profile = valid_profile();
    assert!(DatabaseObjectRef::new(profile, "", "s", "o", DatabaseObjectKind::Table).is_err());
}

#[test]
fn test_3_object_ref_rejects_long_name() {
    let profile = valid_profile();
    let long = "a".repeat(129);
    assert!(DatabaseObjectRef::new(profile, &long, "s", "o", DatabaseObjectKind::Table).is_err());
}

#[test]
fn test_3_object_ref_rejects_control_chars() {
    let profile = valid_profile();
    assert!(
        DatabaseObjectRef::new(profile.clone(), "c\n", "s", "o", DatabaseObjectKind::Table)
            .is_err()
    );
    assert!(
        DatabaseObjectRef::new(profile, "c", "s", "o\u{0}", DatabaseObjectKind::Table).is_err()
    );
}

#[test]
fn test_4_cross_profile_isolation() {
    let profile_a = ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap();
    let profile_b = ProfileIdentity::parse(&format!("p-{}", "b".repeat(64))).unwrap();
    let ref_a =
        DatabaseObjectRef::new(profile_a, "c", "s", "o", DatabaseObjectKind::Table).unwrap();
    let ref_b =
        DatabaseObjectRef::new(profile_b, "c", "s", "o", DatabaseObjectKind::Table).unwrap();
    assert_ne!(ref_a, ref_b);
    let mut set = HashSet::new();
    set.insert(ref_a);
    assert!(!set.contains(&ref_b));
}

#[test]
fn test_5_fingerprint_is_stable() {
    let table = valid_table();
    let fp1 = SchemaFingerprint::of_table(DatabaseObjectKind::Table, &table);
    let fp2 = SchemaFingerprint::of_table(DatabaseObjectKind::Table, &table);
    assert_eq!(fp1, fp2);
}

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
    }
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

fn load_fixture(name: &str) -> SchemaTree {
    let json = match name {
        "postgres.json" => include_str!("fixtures/schemas/postgres.json"),
        "mysql.json" => include_str!("fixtures/schemas/mysql.json"),
        "sqlite.json" => include_str!("fixtures/schemas/sqlite.json"),
        "duckdb.json" => include_str!("fixtures/schemas/duckdb.json"),
        "snowflake.json" => include_str!("fixtures/schemas/snowflake.json"),
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

#[test]
fn test_9_claim_payload_rejects_empty_text() {
    assert!(ClaimPayload::table_description("").is_err());
    assert!(ClaimPayload::table_grain("").is_err());
    assert!(ClaimPayload::column_description("col", "").is_err());
}

#[test]
fn test_9_claim_payload_rejects_long_text() {
    let long = "x".repeat(1025);
    assert!(ClaimPayload::table_description(&long).is_err());
    assert!(ClaimPayload::table_grain(&long).is_err());
    assert!(ClaimPayload::column_description("col", &long).is_err());
}

#[test]
fn test_9_claim_payload_rejects_text_with_newline() {
    assert!(ClaimPayload::table_description("hello\nworld").is_err());
    assert!(ClaimPayload::table_grain("hello\nworld").is_err());
    assert!(ClaimPayload::column_description("col", "hello\nworld").is_err());
}

#[test]
fn test_9_relationship_rejects_mismatched_column_lists() {
    let profile = valid_profile();
    let target = DatabaseObjectRef::new(profile, "c", "s", "t", DatabaseObjectKind::Table).unwrap();
    let result = ClaimPayload::relationship(
        target,
        vec!["a".into(), "b".into()],
        vec!["x".into()],
        Cardinality::OneToOne,
    );
    assert_eq!(result.unwrap_err(), ContractError::ColumnCountMismatch);
}

#[test]
fn test_9_relationship_rejects_empty_column_list() {
    let profile = valid_profile();
    let target = DatabaseObjectRef::new(profile, "c", "s", "t", DatabaseObjectKind::Table).unwrap();
    assert!(
        ClaimPayload::relationship(target, vec![], vec!["x".into()], Cardinality::OneToOne)
            .is_err()
    );
}

#[test]
fn test_9_relationship_rejects_33_columns() {
    let profile = valid_profile();
    let target = DatabaseObjectRef::new(profile, "c", "s", "t", DatabaseObjectKind::Table).unwrap();
    let cols: Vec<String> = (0..=MAX_REFERENCED_COLUMNS)
        .map(|i| format!("c{i}"))
        .collect();
    let result = ClaimPayload::relationship(target, cols.clone(), cols, Cardinality::OneToMany);
    assert_eq!(result.unwrap_err(), ContractError::TooManyColumns);
}

#[test]
fn test_10_origin_may_confirm_directly() {
    use ClaimOrigin::*;
    assert!(UserExplicit.may_confirm_directly());
    // ADR 0002 §4: a reviewed team file enters confirmed within its declared
    // scope, so TeamFile is confirmable without a per-claim review step.
    assert!(TeamFile.may_confirm_directly());
    assert!(!AssistantInferred.may_confirm_directly());
}

#[test]
fn test_10_status_is_recallable() {
    use ClaimStatus::*;
    assert!(Confirmed.is_recallable());
    assert!(!Candidate.is_recallable());
    assert!(!Rejected.is_recallable());
    assert!(!Stale.is_recallable());
    assert!(!Forgotten.is_recallable());
}

#[test]
fn test_11_errors_never_leak_input() {
    let input = "SUPERSECRETVALUE";
    for error in &[
        ContractError::InvalidProfileIdentity,
        ContractError::InvalidClaimId,
        ContractError::EmptyName,
        ContractError::NameTooLong,
        ContractError::ControlCharacter,
        ContractError::EmptyText,
        ContractError::TextTooLong,
        ContractError::TooManyColumns,
        ContractError::ColumnCountMismatch,
        ContractError::EmptyColumns,
    ] {
        let msg = error.to_string();
        assert!(!msg.contains(input), "error {error:?} leaks input");
    }
}

// --- Review additions: fingerprint versioning is what keeps stale claims out, so
// --- a digest must report the format it was built under, not the current one.

#[test]
fn fingerprint_reports_the_version_it_was_built_under() {
    let table = valid_table();
    let live = SchemaFingerprint::of_table(DatabaseObjectKind::Table, &table);
    assert_eq!(live.version(), FINGERPRINT_VERSION);
    assert!(live.is_current_format());

    // A digest loaded from storage under an older format keeps that version.
    let stored = SchemaFingerprint::from_parts(FINGERPRINT_VERSION - 1, live.as_str())
        .expect("valid digest");
    assert_eq!(stored.version(), FINGERPRINT_VERSION - 1);
    assert!(!stored.is_current_format());
    assert_ne!(stored, live);
}

#[test]
fn fingerprint_from_parts_rejects_a_malformed_digest() {
    for bad in ["", "xyz", &"A".repeat(64), &"a".repeat(63), &"a".repeat(65)] {
        assert_eq!(
            SchemaFingerprint::from_parts(FINGERPRINT_VERSION, bad),
            Err(ContractError::InvalidFingerprint),
            "digest {bad:?} should be rejected",
        );
    }
}

#[test]
fn fingerprint_round_trips_through_its_stored_parts() {
    let live = SchemaFingerprint::of_table(DatabaseObjectKind::View, &valid_table());
    let restored =
        SchemaFingerprint::from_parts(live.version(), live.as_str()).expect("valid digest");
    assert_eq!(restored, live);
}
