use super::*;

#[tokio::test]
async fn schema_roundtrip_upsert_invalidate_and_versioned_reopen() {
    let root = temp_root("schema");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store
        .upsert_schema(PROFILE, &schema("events"))
        .await
        .unwrap();
    let cached = store.get_schema(PROFILE).await.unwrap().unwrap();
    assert_eq!(cached.schema, schema("events"));
    assert_eq!(cached.version, 1);
    store
        .upsert_schema(PROFILE, &schema("events_v2"))
        .await
        .unwrap();
    assert_eq!(
        SqliteStateStore::new(&db)
            .get_schema(PROFILE)
            .await
            .unwrap()
            .unwrap()
            .schema,
        schema("events_v2")
    );
    assert_eq!(store.list_schema_metadata().await.unwrap().len(), 1);
    store.invalidate_schema(PROFILE).await.unwrap();
    assert!(store.get_schema(PROFILE).await.unwrap().is_none());
    let _ = fs::remove_dir_all(root);
}
#[tokio::test]
async fn oversized_schema_is_rejected_before_cache_write() {
    let root = temp_root("schema-limit");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let schema = SchemaTree {
        databases: vec![Database {
            name: "main".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![Table {
                    name: "wide".into(),
                    columns: (0..=MAX_SCHEMA_COLUMNS)
                        .map(|index| Column {
                            name: format!("c{index}"),
                            data_type: "TEXT".into(),
                            nullable: true,
                        })
                        .collect(),
                    primary_key: vec![],
                    foreign_keys: vec![],
                }],
            }],
        }],
    };
    assert_eq!(
        store.upsert_schema(PROFILE, &schema).await,
        Err(StoreError::LimitExceeded)
    );
    assert!(store.get_schema(PROFILE).await.unwrap().is_none());
    let _ = fs::remove_dir_all(root);
}
/// A tree that passes structural validation but serializes past the byte
/// cap must still refuse — and cache nothing. Unlike the count-overflow
/// tree above, this one reaches the serialization seam, so it pins the
/// bounded-serializer contract rather than the validator.
#[tokio::test]
async fn oversized_but_valid_schema_tree_is_refused_before_caching() {
    use saya_types::MAX_SCHEMA_BYTES;
    let root = temp_root("schema-bytes");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let wide_name = "n".repeat(200);
    let wide_type = "t".repeat(200);
    let schema = SchemaTree {
        databases: vec![Database {
            name: "main".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![Table {
                    name: "wide".into(),
                    columns: (0..60_000)
                        .map(|index| Column {
                            name: format!("{wide_name}{index:05}"),
                            data_type: wide_type.clone(),
                            nullable: true,
                        })
                        .collect(),
                    primary_key: vec![],
                    foreign_keys: vec![],
                }],
            }],
        }],
    };
    schema
        .validate()
        .expect("the tree must pass structural validation");
    let rendered = serde_json::to_string(&schema).expect("the tree must render");
    assert!(
        rendered.len() > MAX_SCHEMA_BYTES,
        "the fixture must exceed the byte cap: {}",
        rendered.len()
    );
    assert_eq!(
        store.upsert_schema(PROFILE, &schema).await,
        Err(StoreError::LimitExceeded)
    );
    assert!(store.get_schema(PROFILE).await.unwrap().is_none());
    let _ = fs::remove_dir_all(root);
}
