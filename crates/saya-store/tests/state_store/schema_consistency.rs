use super::*;

/// Q2: schema bytes and freshness metadata must come from one consistent
/// read. This pins the source-level guarantee: `get_schema` may run a
/// single statement against `schema_cache`, so no await — and no
/// concurrent upsert or delete — can land between the JSON fetch and the
/// metadata fetch and pair one version's bytes with another's timestamp.
#[test]
fn schema_read_is_a_single_statement_over_the_cache_row() {
    let source = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/schema_store.rs"))
        .expect("schema_store.rs must be readable");
    let body = source
        .split("async fn get_schema")
        .nth(1)
        .expect("get_schema must exist")
        .split("async fn invalidate_schema")
        .next()
        .expect("invalidate_schema must follow get_schema");
    assert_eq!(
        body.matches("FROM schema_cache").count(),
        1,
        "get_schema must read schema bytes and metadata in one statement"
    );
}
/// Q2: the returned freshness metadata always belongs to the returned
/// bytes. A planted row with distinctive metadata must echo back exactly —
/// no old-bytes/new-timestamp pairing from split reads.
#[tokio::test]
async fn schema_read_pairs_bytes_with_their_own_metadata() {
    let root = temp_root("schema-meta-pair");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store
        .upsert_schema(PROFILE, &schema("events"))
        .await
        .unwrap();
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(&db)
                .create_if_missing(true),
        )
        .await
        .unwrap();
    sqlx::query("UPDATE schema_cache SET updated_unix_ms=424242, version=7 WHERE profile_id=?")
        .bind(PROFILE)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let cached = store.get_schema(PROFILE).await.unwrap().unwrap();
    assert_eq!(cached.schema, schema("events"));
    assert_eq!(cached.updated_unix_ms, 424242);
    assert_eq!(cached.version, 7);
    let _ = fs::remove_dir_all(root);
}
/// Q2 hammer: two writers rewrite one cached row while the reader hammers
/// the production `get_schema` read path. Each writer stamps schema bytes
/// and the generation's freshness metadata in ONE atomic statement — the
/// same single-statement upsert production `upsert_schema` runs inside its
/// transaction — so the row on disk only ever holds self-consistent
/// generations: gen-a ⇔ (version 1, odd ms), gen-b ⇔ (version 2, even ms).
/// Every read must pair bytes with metadata from the same generation: a
/// read that pairs one generation's bytes with the other's version or
/// timestamp fails. Public-read controls observe each generation before the
/// contention phase; the concurrent reader checks every pair without
/// assuming scheduler fairness. Each writer ignores the stop flag until it
/// has completed `MIN_WRITES` upserts. The fixed read count and writer cap
/// bound the runtime.
#[tokio::test]
async fn concurrent_upserts_never_tear_schema_from_its_metadata() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    // The public write path computes metadata internally (`now()` and
    // `SCHEMA_VERSION`) and cannot express per-generation metadata, so the
    // writers stamp generations with the production upsert statement over
    // raw pools — the established planted-row pattern of this file.
    const VERSION_A: i64 = 1;
    const VERSION_B: i64 = 2;
    const ODD_BASE_MS: i64 = 1_000_001;
    const EVEN_BASE_MS: i64 = 1_000_000;
    const MIN_WRITES: i64 = 10;
    const MAX_WRITES: i64 = 2_000;
    const READS: usize = 200;

    let root = temp_root("schema-tear");
    let db = root.join("state.sqlite3");
    let store = Arc::new(SqliteStateStore::new(&db));
    assert!(store.get_schema(PROFILE).await.unwrap().is_none());
    let json_a = serde_json::to_string(&schema("gen-a")).unwrap();
    let json_b = serde_json::to_string(&schema("gen-b")).unwrap();
    let pool_a = raw_pool(&db).await;
    let pool_b = raw_pool(&db).await;
    stamp_generation(&pool_a, PROFILE, &json_a, ODD_BASE_MS, VERSION_A)
        .await
        .expect("seed upsert must succeed");
    let cached_a = store.get_schema(PROFILE).await.unwrap().unwrap();
    assert_eq!(cached_a.schema, schema("gen-a"));
    assert_eq!(i64::from(cached_a.version), VERSION_A);
    assert_eq!(cached_a.updated_unix_ms % 2, 1);
    stamp_generation(&pool_b, PROFILE, &json_b, EVEN_BASE_MS, VERSION_B)
        .await
        .expect("generation B control upsert must succeed");
    let cached_b = store.get_schema(PROFILE).await.unwrap().unwrap();
    assert_eq!(cached_b.schema, schema("gen-b"));
    assert_eq!(i64::from(cached_b.version), VERSION_B);
    assert_eq!(cached_b.updated_unix_ms % 2, 0);

    let stop = Arc::new(AtomicBool::new(false));
    let writer_a = {
        let stop = Arc::clone(&stop);
        let json = json_a.clone();
        let pool = pool_a.clone();
        tokio::spawn(async move {
            let mut index: i64 = 0;
            // A writer ignores the stop flag until it has completed
            // MIN_WRITES upserts, so the post-join progress assertion can
            // never pass while a writer starved below the bound.
            while (!stop.load(Ordering::Relaxed) || index < MIN_WRITES) && index < MAX_WRITES {
                stamp_generation(&pool, PROFILE, &json, ODD_BASE_MS + 2 * index, VERSION_A)
                    .await
                    .expect("writer A upsert must succeed");
                index += 1;
            }
            index
        })
    };
    let writer_b = {
        let stop = Arc::clone(&stop);
        let json = json_b.clone();
        let pool = pool_b.clone();
        tokio::spawn(async move {
            let mut index: i64 = 0;
            while (!stop.load(Ordering::Relaxed) || index < MIN_WRITES) && index < MAX_WRITES {
                stamp_generation(&pool, PROFILE, &json, EVEN_BASE_MS + 2 * index, VERSION_B)
                    .await
                    .expect("writer B upsert must succeed");
                index += 1;
            }
            index
        })
    };
    let (mut saw_a, mut saw_b) = (0_u32, 0_u32);
    for _ in 0..READS {
        match store.get_schema(PROFILE).await {
            Ok(Some(cached)) => {
                let table = &cached.schema.databases[0].schemas[0].tables[0].name;
                match table.as_str() {
                    "gen-a" => {
                        saw_a += 1;
                        assert_eq!(
                            i64::from(cached.version),
                            VERSION_A,
                            "gen-a bytes paired with version {}",
                            cached.version
                        );
                        assert_eq!(
                            cached.updated_unix_ms % 2,
                            1,
                            "gen-a bytes paired with timestamp {}",
                            cached.updated_unix_ms
                        );
                    }
                    "gen-b" => {
                        saw_b += 1;
                        assert_eq!(
                            i64::from(cached.version),
                            VERSION_B,
                            "gen-b bytes paired with version {}",
                            cached.version
                        );
                        assert_eq!(
                            cached.updated_unix_ms % 2,
                            0,
                            "gen-b bytes paired with timestamp {}",
                            cached.updated_unix_ms
                        );
                    }
                    other => panic!("torn schema bytes: {other}"),
                }
            }
            Ok(None) => {}
            Err(error) => panic!("a live row must read cleanly: {error:?}"),
        }
    }
    stop.store(true, Ordering::Relaxed);
    let wrote_a = writer_a.await.unwrap();
    let wrote_b = writer_b.await.unwrap();
    assert!(
        saw_a + saw_b >= 1,
        "the contended reader must observe a live generation"
    );
    assert!(
        wrote_a >= MIN_WRITES,
        "writer A completed only {wrote_a} upserts"
    );
    assert!(
        wrote_b >= MIN_WRITES,
        "writer B completed only {wrote_b} upserts"
    );
    pool_a.close().await;
    pool_b.close().await;
    let _ = fs::remove_dir_all(root);
}
