#[test]
fn compact_schema_qualifies_keys_and_prevents_collisions() {
    let schema_tree = SchemaTree {
        databases: vec![Database {
            name: "main".to_string(),
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

    let compact = compact_schema(&schema_tree, SqlDialect::Postgres);
    let tables = compact.get("tables").unwrap().as_object().unwrap();

    assert_eq!(tables.len(), 2);
    assert_eq!(tables.get("main.public.users").unwrap(), "id:INTEGER");
    assert_eq!(tables.get("main.sales.users").unwrap(), "email:TEXT");
}

#[test]
fn compact_schema_single_table_uses_fully_qualified_key() {
    let schema_tree = SchemaTree {
        databases: vec![Database {
            name: "db1".to_string(),
            schemas: vec![Schema {
                name: "schema1".to_string(),
                tables: vec![Table {
                    name: "orders".to_string(),
                    columns: vec![
                        Column {
                            name: "id".to_string(),
                            data_type: "INT".to_string(),
                            nullable: false,
                        },
                        Column {
                            name: "amount".to_string(),
                            data_type: "NUMERIC".to_string(),
                            nullable: true,
                        },
                    ],
                    primary_key: vec![],
                    foreign_keys: vec![],
                }],
            }],
        }],
    };

    let compact = compact_schema(&schema_tree, SqlDialect::Postgres);
    let tables = compact.get("tables").unwrap().as_object().unwrap();

    assert_eq!(tables.len(), 1);
    assert_eq!(
        tables.get("db1.schema1.orders").unwrap(),
        "id:INT, amount:NUMERIC"
    );
}

/// The schema handed to the model is also the example it copies. Keyed
/// `database.schema.table` on SQLite — which parses neither part — the model
/// writes back the three-part name it was shown and the statement is rejected,
/// so it pays a wasted round trip before retrying unqualified. The key must be
/// a name the engine will actually accept.
#[test]
fn compact_schema_keys_are_names_the_engine_accepts() {
    let tree = SchemaTree {
        databases: vec![Database {
            name: "concert_singer".to_string(),
            schemas: vec![Schema {
                name: "main".to_string(),
                tables: vec![Table {
                    name: "singer".to_string(),
                    columns: vec![Column {
                        name: "id".to_string(),
                        data_type: "INTEGER".to_string(),
                        nullable: false,
                    }],
                    primary_key: vec![],
                    foreign_keys: vec![],
                }],
            }],
        }],
    };

    let sqlite = compact_schema(&tree, SqlDialect::Sqlite);
    let tables = sqlite.get("tables").unwrap().as_object().unwrap();
    assert!(
        tables.contains_key("singer"),
        "SQLite takes a bare table name, got {:?}",
        tables.keys().collect::<Vec<_>>()
    );

    let mysql = compact_schema(&tree, SqlDialect::Mysql);
    let tables = mysql.get("tables").unwrap().as_object().unwrap();
    assert!(
        tables.contains_key("concert_singer.singer"),
        "MySQL qualifies by database only, got {:?}",
        tables.keys().collect::<Vec<_>>()
    );

    let postgres = compact_schema(&tree, SqlDialect::Postgres);
    let tables = postgres.get("tables").unwrap().as_object().unwrap();
    assert!(
        tables.contains_key("concert_singer.main.singer"),
        "PostgreSQL keeps the three-part name, got {:?}",
        tables.keys().collect::<Vec<_>>()
    );
}
