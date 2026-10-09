use super::*;

/// Spec test 2 (byte bound): contracts the byte bound drops from the end are
/// counted. With a tiny byte budget that admits fewer than the matched objects,
/// the receipt counts the dropped contracts' claims.
#[tokio::test]
async fn claims_dropped_by_the_byte_bound_are_counted() {
    let root = temp_root("p1a_dropped_byte");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    // Five objects, each with one ~1 KiB description claim, all `current`.
    let tables: Vec<Table> = (0..5)
        .map(|i| table_named_with(&format!("orders{i}"), &[("id", "bigint", false)]))
        .collect();
    let tree = SchemaTree {
        databases: vec![Database {
            name: "catalog".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables,
            }],
        }],
    };
    store.upsert_schema(identity.as_str(), &tree).await.unwrap();
    let big = "z".repeat(1024);
    for i in 0..5 {
        let obj = object(&identity, &format!("orders{i}"));
        let _ = live_fingerprint(&table_named_with(
            &format!("orders{i}"),
            &[("id", "bigint", false)],
        ));
        put_item(
            &store,
            &obj,
            ClaimPayload::table_description(&big).unwrap(),
            KnowledgeState::Active,
        )
        .await;
    }
    let registry = registry_for("analytics", &identity);
    // A budget that admits a couple of the 1 KiB stanzas but not all five.
    let bounds = RecallBounds {
        max_objects: 5,
        max_claims_per_object: 12,
        max_bytes: 2300,
    };
    let (blocks, receipt) = recall_blocks(
        "orders",
        None,
        true,
        RecallMode::Confirmed,
        bounds,
        &registry,
        Some(&store),
    )
    .await;
    let _ = &blocks;
    // The byte bound dropped at least one whole contract (its claim), so the
    // count is non-zero and matches the kept-vs-selected gap.
    let supplied_claims: usize = receipt.supplied.iter().map(|c| c.claims.len()).sum();
    assert!(supplied_claims < 5, "the byte bound dropped contracts");
    assert_eq!(
        receipt.dropped_by_bounds,
        5 - supplied_claims,
        "dropped claims = (5 selected) − (supplied): {:?}",
        receipt
    );
    let _ = fs::remove_dir_all(root);
}

/// Spec test 3: `recall = off` / privacy-gate-closed is distinguishable from
/// Spec test 3: `recall = off` / privacy-gate-closed is distinguishable from
/// "recall ran and found nothing". The privacy-gate-closed path returns a
/// `PrivacyGateClosed` receipt; a prompt that matches nothing returns a `Ran` receipt
/// with empty `supplied`. The two must not be confusable. (The `recall = off`
/// arm is a runtime concern producing `ConfiguredOff`; this module's
/// `recall_context_blocks` is never called with recall off, so it cannot observe it —
/// but the `PrivacyGateClosed` variant it constructs is distinct from `ConfiguredOff`
/// and `Ran`.)
#[tokio::test]
async fn skipped_is_distinguishable_from_ran_and_found_nothing() {
    use crate::contracts::{RecallOutcomeKind, RecallReceipt};

    // Privacy gate closed: the function returns before any store query, with a
    // PrivacyGateClosed receipt. The store is intentionally unopenable to prove it was
    // never queried: had it been, the call would error rather than skip.
    let root = temp_root("p1a_skipped");
    fs::write(root.join("blocker"), b"x").unwrap();
    let bad_path = root.join("blocker/state.sqlite3");
    let unopenable = SqliteStateStore::new(&bad_path);
    let identity = identity_for("analytics");
    let registry = registry_for("analytics", &identity);
    let (_blocks, skipped) = recall_blocks(
        "orders by month",
        None,
        false, // privacy gate closed
        RecallMode::Confirmed,
        RecallBounds::defaults(),
        &registry,
        Some(&unopenable),
    )
    .await;
    assert_eq!(skipped.kind, RecallOutcomeKind::PrivacyGateClosed);
    assert!(skipped.supplied.is_empty());
    assert_eq!(skipped.dropped_by_bounds, 0);
    let _ = fs::remove_dir_all(root);

    // Ran and found nothing: a prompt matching nothing yields a Ran receipt with
    // empty supplied — the same shape as PrivacyGateClosed's, but a different `kind`.
    let root2 = temp_root("p1a_ran_empty");
    let db2 = root2.join("state.sqlite3");
    let store2 = store_at(&db2, &identity).await;
    let (obj, fp) = seed_orders_with_created_at(&store2, &identity).await;
    let _ = (obj, fp);
    let registry2 = registry_for("analytics", &identity);
    let (_blocks, ran_empty) = recall_blocks(
        "completely unrelated zzztop words",
        None,
        true,
        RecallMode::Confirmed,
        RecallBounds::defaults(),
        &registry2,
        Some(&store2),
    )
    .await;
    assert_eq!(
        ran_empty.kind,
        RecallOutcomeKind::Ran {
            store_unavailable: false
        }
    );
    assert!(ran_empty.supplied.is_empty());
    // The two are distinguishable: PrivacyGateClosed ≠ Ran.
    assert_ne!(skipped.kind, ran_empty.kind);

    // Configured off is also distinguishable: ConfiguredOff ≠ PrivacyGateClosed ≠ Ran.
    let configured_off = RecallReceipt::configured_off();
    assert_eq!(configured_off.kind, RecallOutcomeKind::ConfiguredOff);
    assert_ne!(configured_off.kind, skipped.kind);
    assert_ne!(configured_off.kind, ran_empty.kind);

    let _ = fs::remove_dir_all(root2);
}

/// Spec test 4: a candidate claim's status survives into the receipt as
/// `Candidate`, not flattened to a single "included" notion — P1b renders it
/// differently and P3 acts on it.
#[tokio::test]
async fn a_candidate_claims_status_survives_into_the_receipt() {
    let root = temp_root("p1a_candidate_status");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    let obj = object(&identity, "orders");
    let tree = orders_schema(&identity);
    store
        .upsert_schema(identity.as_str(), &tree.1)
        .await
        .unwrap();
    let fp = live_fingerprint(&orders_table());
    remember_candidate_default_time_column(&store, &obj, &fp, "created_at").await;
    let registry = registry_for("analytics", &identity);

    let (blocks, receipt) = recall_blocks(
        "orders by month",
        None,
        true,
        RecallMode::IncludeCandidates,
        RecallBounds::defaults(),
        &registry,
        Some(&store),
    )
    .await;
    let _ = &blocks;
    assert_eq!(receipt.supplied.len(), 1);
    assert_eq!(receipt.supplied[0].claims.len(), 1);
    assert_eq!(
        receipt.supplied[0].claims[0].status,
        ClaimStatus::Candidate,
        "a candidate survives as Candidate, not flattened to confirmed/included"
    );
    let _ = fs::remove_dir_all(root);
}
