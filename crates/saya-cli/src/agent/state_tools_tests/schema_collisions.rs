/// Shortening the key must not merge two different tables into one entry. When
/// the engine's own depth cannot tell them apart, the fully qualified name is
/// used instead — a name the model cannot run is still better than a schema
/// that hides a table.
#[test]
fn a_shortened_key_never_collapses_two_tables() {
    let tree = SchemaTree {
        databases: vec![Database {
            name: "db".to_string(),
            schemas: vec![
                Schema {
                    name: "public".to_string(),
                    tables: vec![Table {
                        name: "users".to_string(),
                        columns: vec![Column {
                            name: "id".to_string(),
                            data_type: "INTEGER".to_string(),
                            nullable: false,
                        }],
                        primary_key: vec![],
                        foreign_keys: vec![],
                    }],
                },
                Schema {
                    name: "sales".to_string(),
                    tables: vec![Table {
                        name: "users".to_string(),
                        columns: vec![Column {
                            name: "email".to_string(),
                            data_type: "TEXT".to_string(),
                            nullable: true,
                        }],
                        primary_key: vec![],
                        foreign_keys: vec![],
                    }],
                },
            ],
        }],
    };

    let compact = compact_schema(&tree, SqlDialect::Sqlite);
    let tables = compact.get("tables").unwrap().as_object().unwrap();
    assert_eq!(tables.len(), 2, "both tables must survive: {tables:?}");
}

/// The schema the model reads each turn is a single string per table, so the
/// primary-key and foreign-key additions are pinned byte-for-byte: a silent
/// change to this shape changes what the model writes without anyone noticing.
/// Composite keys render with parenthesised column lists so a reader can pair
/// local and referenced columns positionally.
#[test]
fn compact_schema_renders_primary_keys_and_foreign_keys_compactly() {
    let tree = SchemaTree {
        databases: vec![Database {
            name: "db".to_string(),
            schemas: vec![Schema {
                name: "public".to_string(),
                tables: vec![
                    Table {
                        name: "customers".to_string(),
                        columns: vec![
                            Column {
                                name: "id".to_string(),
                                data_type: "INT".to_string(),
                                nullable: false,
                            },
                            Column {
                                name: "name".to_string(),
                                data_type: "TEXT".to_string(),
                                nullable: true,
                            },
                        ],
                        primary_key: vec!["id".to_string()],
                        foreign_keys: vec![],
                    },
                    Table {
                        name: "orders".to_string(),
                        columns: vec![
                            Column {
                                name: "id".to_string(),
                                data_type: "INT".to_string(),
                                nullable: false,
                            },
                            Column {
                                name: "customer_id".to_string(),
                                data_type: "INT".to_string(),
                                nullable: false,
                            },
                            Column {
                                name: "amount".to_string(),
                                data_type: "NUMERIC".to_string(),
                                nullable: true,
                            },
                        ],
                        primary_key: vec!["id".to_string()],
                        foreign_keys: vec![ForeignKey {
                            columns: vec!["customer_id".to_string()],
                            referenced_schema: None,
                            referenced_table: "customers".to_string(),
                            referenced_columns: vec!["id".to_string()],
                        }],
                    },
                    Table {
                        name: "composite_parent".to_string(),
                        columns: vec![
                            Column {
                                name: "a".to_string(),
                                data_type: "INT".to_string(),
                                nullable: true,
                            },
                            Column {
                                name: "b".to_string(),
                                data_type: "INT".to_string(),
                                nullable: true,
                            },
                        ],
                        primary_key: vec!["a".to_string(), "b".to_string()],
                        foreign_keys: vec![],
                    },
                    Table {
                        name: "composite_child".to_string(),
                        columns: vec![
                            Column {
                                name: "id".to_string(),
                                data_type: "INT".to_string(),
                                nullable: false,
                            },
                            Column {
                                name: "a".to_string(),
                                data_type: "INT".to_string(),
                                nullable: true,
                            },
                            Column {
                                name: "b".to_string(),
                                data_type: "INT".to_string(),
                                nullable: true,
                            },
                        ],
                        primary_key: vec!["id".to_string()],
                        foreign_keys: vec![ForeignKey {
                            columns: vec!["a".to_string(), "b".to_string()],
                            referenced_schema: None,
                            referenced_table: "composite_parent".to_string(),
                            referenced_columns: vec!["a".to_string(), "b".to_string()],
                        }],
                    },
                ],
            }],
        }],
    };

    let compact = compact_schema(&tree, SqlDialect::Sqlite);
    let tables = compact.get("tables").unwrap().as_object().unwrap();
    assert_eq!(
        tables.get("customers").unwrap(),
        "id:INT, name:TEXT | pk=id"
    );
    assert_eq!(
        tables.get("composite_parent").unwrap(),
        "a:INT, b:INT | pk=a,b"
    );
    assert_eq!(
        tables.get("orders").unwrap(),
        "id:INT, customer_id:INT, amount:NUMERIC | pk=id | fk=customer_id->customers.id"
    );
    assert_eq!(
        tables.get("composite_child").unwrap(),
        "id:INT, a:INT, b:INT | pk=id | fk=(a,b)->composite_parent.(a,b)"
    );
}

/// A foreign key pointing through a name that two tables share must name the
/// target by the same key it appears under, or the model joins the wrong table.
#[test]
fn foreign_key_reference_uses_the_full_key_when_the_target_name_collides() {
    let tree = SchemaTree {
        databases: vec![Database {
            name: "db".to_string(),
            schemas: vec![
                Schema {
                    name: "auth".to_string(),
                    tables: vec![Table {
                        name: "users".to_string(),
                        columns: vec![Column {
                            name: "id".to_string(),
                            data_type: "INT".to_string(),
                            nullable: false,
                        }],
                        primary_key: vec!["id".to_string()],
                        foreign_keys: vec![],
                    }],
                },
                Schema {
                    name: "sales".to_string(),
                    tables: vec![Table {
                        name: "users".to_string(),
                        columns: vec![
                            Column {
                                name: "id".to_string(),
                                data_type: "INT".to_string(),
                                nullable: false,
                            },
                            Column {
                                name: "manager_id".to_string(),
                                data_type: "INT".to_string(),
                                nullable: true,
                            },
                        ],
                        primary_key: vec!["id".to_string()],
                        foreign_keys: vec![ForeignKey {
                            columns: vec!["manager_id".to_string()],
                            referenced_schema: Some("auth".to_string()),
                            referenced_table: "users".to_string(),
                            referenced_columns: vec!["id".to_string()],
                        }],
                    }],
                },
            ],
        }],
    };

    let compact = compact_schema(&tree, SqlDialect::Postgres);
    let tables = compact.get("tables").unwrap().as_object().unwrap();
    assert_eq!(
        tables.len(),
        2,
        "both users tables keep full keys: {tables:?}"
    );
    let sales = tables.get("db.sales.users").unwrap();
    let sales = sales.as_str().expect("table value is a string");
    assert!(
        sales.contains("manager_id->db.auth.users.id"),
        "FK must resolve to the target's full key, got: {sales}"
    );
}
