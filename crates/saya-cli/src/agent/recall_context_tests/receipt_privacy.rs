use super::*;

/// Spec test 5: store-unavailable yields a receipt (not an error) and the turn
/// still completes. The receipt records the failure as `Ran { store_unavailable:
/// true }` with empty `supplied`, so a later phase can name it without an error.
#[tokio::test]
async fn store_unavailable_yields_a_receipt_not_an_error() {
    use crate::contracts::RecallOutcomeKind;
    let root = temp_root("p1a_store_unavailable");
    fs::write(root.join("blocker"), b"x").unwrap();
    let bad_path = root.join("blocker/state.sqlite3");
    let store = SqliteStateStore::new(&bad_path); // parent is a file → unopenable
    let identity = identity_for("analytics");
    let registry = registry_for("analytics", &identity);

    // The turn completes: the call returns a receipt rather than panicking or
    // propagating an error.
    let (blocks, receipt) = recall_blocks(
        "orders by month",
        None,
        true,
        RecallMode::Confirmed,
        RecallBounds::defaults(),
        &registry,
        Some(&store),
    )
    .await;
    assert!(blocks.is_empty(), "no block when the store is unavailable");
    assert_eq!(
        receipt.kind,
        RecallOutcomeKind::Ran {
            store_unavailable: true
        }
    );
    assert!(receipt.supplied.is_empty());
    let _ = fs::remove_dir_all(root);
}

/// Spec test 6: no `ProfileIdentity` value appears in the receipt. The opaque
/// identity is a hash over connection material; the receipt carries the
/// human-facing name only. Asserted both at runtime (the identity string is
/// absent from the receipt's Debug output) and structurally — the receipt types
/// in `contracts/receipt.rs` have no `ProfileIdentity` field, by construction.
#[tokio::test]
async fn no_opaque_profile_identity_value_appears_in_the_receipt() {
    let root = temp_root("p1a_no_identity");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    seed_orders_with_created_at(&store, &identity).await;
    let registry = registry_for("analytics", &identity);

    let (_blocks, receipt) = recall_blocks(
        "orders by month",
        None,
        true,
        RecallMode::Confirmed,
        RecallBounds::defaults(),
        &registry,
        Some(&store),
    )
    .await;
    // The opaque identity string appears nowhere in the receipt's Debug output.
    let debug = format!("{receipt:?}");
    assert!(
        !debug.contains(identity.as_str()),
        "opaque identity leaked into the receipt: {debug}"
    );
    // The human-facing name does appear, in place of the identity.
    assert!(
        receipt.supplied.iter().any(|c| c.profile == "analytics"),
        "the profile name (not the identity) is what the receipt carries: {debug}"
    );
    let _ = fs::remove_dir_all(root);
}
