#[tokio::test]
async fn queue_reports_current_against_a_cached_schema() {
    let root = temp_root("queue_cached_schema_state");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;
    // Cache the schema that matches the seeded candidate — what
    // `connection schema <profile> --refresh` would have written.
    let identity = identity_for(&runtime, "local");
    store
        .upsert_schema(&identity, &orders_schema())
        .await
        .unwrap();
    seed_current_candidate(&store, &runtime).await;

    let queue = ContractsCommand::Queue {
        profile: None,
        limit: None,
    };
    let (code, out, err) = run(queue, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "queue stderr: {err}");
    assert!(
        out.contains("[current]"),
        "queue must report current against the cached schema: {out}"
    );
    assert!(
        !out.contains("live_schema_unavailable"),
        "queue must not report live_schema_unavailable when a schema is cached: {out}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 17. The queue must not present an unreviewed proposal and a fact whose
//     dependency broke as the same thing. Under the new model staleness is
//     computed, not persisted: a "stale" fact is `Active` (state) whose
//     `SchemaBinding` reads `Invalid` against the cached schema. The queue
//     lists `Pending` candidates only, so the broken confirmed fact is not in
//     the queue — it is a different kind of thing, surfaced by `show` as
//     `stale`. The two are distinguishable: the candidate is queued (status
//     `candidate`), the broken fact is not queued and `show` reports it stale.
// ---------------------------------------------------------------------------
/// Seeds an `Active` `default_time_column` fact on `created_at` for `table`,
/// then caches a schema for the profile that has `table` but dropped
/// `created_at` — so the item's `Column { created_at, Time }` binding computes
/// `Invalid` against the cache. This is the new-model "stale" shape: a
/// confirmed fact whose dependency broke. Returns the item's `ki-…` id.
async fn seed_drifted_active_claim(
    store: &SqliteStateStore,
    runtime: &RuntimeConfig,
    table: &str,
) -> ClaimId {
    let identity = identity_for(runtime, "local");
    let profile = ProfileIdentity::parse(&identity).unwrap();
    let object = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        table,
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let payload = ClaimPayload::default_time_column("created_at", None).unwrap();
    let slot = KnowledgeSlot::TableDefaultTime;
    let binding = SchemaBinding::derive(&slot, &payload).expect("slot/payload agree");
    store
        .put_knowledge_item(KnowledgeItemRequest {
            object: object.clone(),
            slot,
            value: payload,
            source: ClaimOrigin::UserExplicit,
            state: KnowledgeState::Active,
            schema_binding_json: serde_json::to_string(&binding).unwrap(),
            fingerprint: SchemaFingerprint::from_parts(FINGERPRINT_VERSION, &"0".repeat(64))
                .unwrap(),
        })
        .await
        .unwrap();
    // Cache a schema that has `table` but dropped `created_at`, so the item's
    // `Column { created_at, Time }` binding reads `Invalid` (→ `stale`).
    let drifted = SchemaTree {
        databases: vec![Database {
            name: "analytics".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![Table {
                    name: table.into(),
                    primary_key: vec![],
                    foreign_keys: vec![],
                    columns: vec![Column {
                        name: "id".into(),
                        data_type: "bigint".into(),
                        nullable: false,
                    }],
                }],
            }],
        }],
    };
    store.upsert_schema(&identity, &drifted).await.unwrap();
    let id = store
        .knowledge_for_object(&object)
        .await
        .expect("knowledge items listed")
        .into_iter()
        .find(|i| i.slot == KnowledgeSlot::TableDefaultTime)
        .expect("drifted item stored")
        .id;
    ClaimId::parse(&id).expect("ki id parses")
}

#[tokio::test]
async fn queue_distinguishes_candidate_from_stale_in_output() {
    let root = temp_root("queue_candidate_vs_stale");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;
    // A `Pending` candidate (an unreviewed proposal) and an `Active` fact whose
    // binding computes `Invalid` (a confirmed fact whose dependency broke). The
    // queue is the candidate worklist: the candidate is queued; the broken
    // confirmed fact is not — it is surfaced by `show` as `stale`, not conflated
    // with a candidate.
    let cand_id = seed_candidate(&store, &runtime, "orders").await;
    let stale_id = seed_drifted_active_claim(&store, &runtime, "shipments").await;

    let queue = ContractsCommand::Queue {
        profile: None,
        limit: None,
    };
    let (code, out, err) = run(queue.clone(), &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "queue stderr: {err}");

    // The candidate is queued; the broken confirmed fact is not. The queue
    // does not present them as the same thing — one is a candidate to decide
    // on, the other is a drifted fact that is not pending review.
    assert!(
        out.contains(cand_id.as_str()),
        "candidate id missing from queue: {out}"
    );
    assert!(
        !out.contains(stale_id.as_str()),
        "a broken confirmed fact must not appear in the candidate queue: {out}"
    );
    assert!(
        out.contains("candidate"),
        "queue must show the candidate's status: {out}"
    );

    // `show` surfaces the broken fact as `stale` — the dependency-broke verdict
    // a reviewer acts on — so the two are told apart across the two views: the
    // queue has the candidate, `show` has the broken fact as stale. The show
    // stanza abbreviates the claim id (`ki-f35…`), so assert on the value the
    // broken fact carries and its stale state rather than the full id.
    let (code, out, err) = run(
        ContractsCommand::Show {
            table: "analytics.public.shipments".into(),
            profile: None,
        },
        &runtime,
        &store,
        RenderFormat::Text,
    )
    .await;
    assert_eq!(code, 0, "show stderr: {err}");
    assert!(
        out.contains("created_at"),
        "the broken fact is shown to a reviewer: {out}"
    );
    assert!(
        out.contains("[stale]"),
        "show reports the broken fact as stale, not current: {out}"
    );

    // The JSON queue carries the candidate's status so a machine reader sees
    // the same distinction.
    let (code, out, err) = run(queue, &runtime, &store, RenderFormat::Json).await;
    assert_eq!(code, 0, "queue json stderr: {err}");
    let events = json_events(&out);
    let queue_event = events
        .iter()
        .find(|v| v["event"] == "contract_queue")
        .expect("a contract_queue event was emitted");
    let items = queue_event["items"].as_array().expect("items is an array");
    let cand_item = items
        .iter()
        .find(|v| v["claim_id"] == cand_id.as_str())
        .expect("candidate item present in JSON");
    assert_eq!(
        cand_item["status"], "candidate",
        "JSON candidate status: {cand_item}"
    );
    assert!(
        !items.iter().any(|v| v["claim_id"] == stale_id.as_str()),
        "the broken confirmed fact must not be a queue item: {items:?}"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// the `decide` command covers every capability `review` had, so
// retiring `review` loses nothing. Two differences matter:
//   (a) `review` took a full claim id; `decide` takes a prefix with a
//       `MIN_PREFIX_LEN` floor. A full-length id is a prefix of itself, so it
//       must resolve to exactly one claim.
//   (b) `review` was not profile-scoped; `decide` resolves the prefix against
//       the resolved profile's claims and has a `--profile` flag. A claim in a
//       non-active profile must stay reachable via `decide --profile <name>`.
// These two tests are the evidence; they are written against the *current*
// binary (before anything is removed) and must pass both before and after.
// ---------------------------------------------------------------------------
