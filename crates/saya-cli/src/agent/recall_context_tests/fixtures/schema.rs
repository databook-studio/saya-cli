fn object(identity: &ProfileIdentity, name: &str) -> DatabaseObjectRef {
    DatabaseObjectRef::new(
        identity.clone(),
        "catalog",
        "public",
        name,
        DatabaseObjectKind::Table,
    )
    .unwrap()
}

/// A live schema with the orders table carrying `created_at`, matching the
/// stored fingerprint so the claim reads `current`.
fn orders_schema(identity: &ProfileIdentity) -> (ProfileIdentity, SchemaTree) {
    let table = Table {
        name: "orders".into(),
        primary_key: vec![],
        foreign_keys: vec![],
        columns: vec![
            Column {
                name: "id".into(),
                data_type: "bigint".into(),
                nullable: false,
            },
            Column {
                name: "created_at".into(),
                data_type: "timestamp".into(),
                nullable: false,
            },
        ],
    };
    let tree = SchemaTree {
        databases: vec![Database {
            name: "catalog".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![table],
            }],
        }],
    };
    (identity.clone(), tree)
}

/// The fingerprint the `unobserved` headless path would have stored under;
/// a `default_time_column` claim made live with this schema reads `current`.
fn live_fingerprint(tree: &Table) -> saya_types::SchemaFingerprint {
    saya_types::SchemaFingerprint::of_table(DatabaseObjectKind::Table, tree)
}
