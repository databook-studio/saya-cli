use super::*;

// P2a — a confirmed claim binds; a candidate does not; deviation is declared.
//
// A confirmed claim gains a stanza-level directive (one per contract, before
// its claims) and a `[confirmed] ` point-of-use marker; a candidate keeps its
// `[candidate — unconfirmed] ` marker and gains no authority. A disputed
// confirmed claim shows `[disputed] ` and not `[confirmed] `, so a
// disagreement never reads as a settled instruction. These tests pin the
// asymmetry: each fails if the directive or the confirmed marker is removed,
// or if a candidate or a disputed claim is raised to binding.
// ===========================================================================

/// D4a: a confirmed claim renders with the stanza directive and the
/// `[confirmed] ` marker. Removing either the directive line or the marker
/// breaks the assertions, so the test fails if the binding force is dropped.
#[tokio::test]
async fn confirmed_claim_renders_with_directive_and_marker() {
    let root = temp_root("p2a_confirmed_marker");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    seed_orders_with_created_at(&store, &identity).await;

    let registry = registry_for("analytics", &identity);
    let (blocks, _receipt) = recall_blocks(
        "orders by month",
        None,
        true,
        RecallMode::Confirmed,
        RecallBounds::defaults(),
        &registry,
        Some(&store),
    )
    .await;
    assert_eq!(blocks.len(), 1);
    let body = &blocks[0].body;
    // The stanza directive is present (one line, naming "Confirmed").
    assert!(
        body.contains(super::render::CONFIRMED_DIRECTIVE),
        "stanza directive is present: {body}"
    );
    // The confirmed claim carries the in-band marker, not a bare line.
    assert!(
        body.contains("[confirmed] "),
        "confirmed claim carries the [confirmed] marker: {body}"
    );
    // The directive precedes the claim line: "bind" appears before "default_time_column".
    let directive_idx = body.find(super::render::CONFIRMED_DIRECTIVE).unwrap();
    let marker_idx = body.find("[confirmed] ").unwrap();
    assert!(
        directive_idx < marker_idx,
        "directive precedes the confirmed claim line: {body}"
    );
    let _ = fs::remove_dir_all(root);
}

/// D4b: a candidate claim does NOT read as binding — it keeps its
/// `[candidate — unconfirmed] ` marker and carries no `[confirmed] ` marker.
/// Under `include-candidates` with only a candidate, no confirmed claim is
/// present, so the `[confirmed]` marker never appears.
#[tokio::test]
async fn candidate_claim_does_not_read_as_binding_and_keeps_its_marker() {
    let root = temp_root("p2a_candidate_not_binding");
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
    let (blocks, _receipt) = recall_blocks(
        "orders by month",
        None,
        true,
        RecallMode::IncludeCandidates,
        RecallBounds::defaults(),
        &registry,
        Some(&store),
    )
    .await;
    assert_eq!(blocks.len(), 1);
    let body = &blocks[0].body;
    // The candidate keeps its existing marker, unchanged.
    assert!(
        body.contains("[candidate — unconfirmed] "),
        "candidate keeps its marker: {body}"
    );
    // No confirmed claim is present, so no confirmed marker appears — a
    // candidate must not read as binding.
    assert!(
        !body.contains("[confirmed] "),
        "candidate stanza carries no confirmed marker: {body}"
    );
    let _ = fs::remove_dir_all(root);
}

/// D4c: a confirmed claim and a candidate claim in the same stanza stay
/// distinguishable — one carries `[confirmed] `, the other
/// `[candidate — unconfirmed] `, so a reader can tell which has authority.
#[tokio::test]
async fn confirmed_and_candidate_in_one_stanza_remain_distinguishable() {
    let root = temp_root("p2a_distinguishable");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    seed_orders_confirmed_and_candidate(&store, &identity).await;

    let registry = registry_for("analytics", &identity);
    let (blocks, _receipt) = recall_blocks(
        "orders by month",
        None,
        true,
        RecallMode::IncludeCandidates,
        RecallBounds::defaults(),
        &registry,
        Some(&store),
    )
    .await;
    assert_eq!(blocks.len(), 1);
    let body = &blocks[0].body;
    // Both markers appear, on different lines: the confirmed alias and the
    // candidate default-time-column are distinguishable at the point of use.
    assert!(
        body.contains("[confirmed] "),
        "confirmed alias carries the confirmed marker: {body}"
    );
    assert!(
        body.contains("[candidate — unconfirmed] "),
        "candidate carries the candidate marker: {body}"
    );
    let confirmed_lines = body.lines().filter(|l| l.contains("[confirmed] ")).count();
    let candidate_lines = body
        .lines()
        .filter(|l| l.contains("[candidate — unconfirmed] "))
        .count();
    assert_eq!(confirmed_lines, 1, "exactly one confirmed line: {body}");
    assert_eq!(candidate_lines, 1, "exactly one candidate line: {body}");
    let _ = fs::remove_dir_all(root);
}

/// D4d: a disputed confirmed claim does NOT read as binding. Two
/// contradictory confirmed `table_grain` claims both carry `[disputed] ` and
/// NOT `[confirmed] ` — the dispute marker wins precedence, so
/// a disagreement is never presented as a settled instruction.
#[tokio::test]
async fn disputed_confirmed_claim_does_not_read_as_binding() {
    use crate::contracts::{ContractClaim, ContractConflict, RetrievedContract};
    // D-3 NOTE: two confirmed grains cannot coexist in `knowledge_items`; see
    // `conflicting_grains_both_appear_marked_and_kind_named`. Render a
    // directly-constructed conflict contract so the behaviour it guards — a
    // disputed confirmed claim carries `[disputed]`, not `[confirmed]` — holds.
    let identity = identity_for("analytics");
    let obj = object(&identity, "orders");
    let contract = RetrievedContract {
        object: obj.clone(),
        schema_state: crate::contracts::ContractSchemaState::Current,
        claims: vec![
            ContractClaim {
                id: ClaimId::parse("c-grain0001").unwrap(),
                object: obj.clone(),
                value: ClaimPayload::table_grain("one row per order", None).unwrap(),
                source: ClaimOrigin::UserExplicit,
                status: ClaimStatus::Confirmed,
            },
            ContractClaim {
                id: ClaimId::parse("c-grain0002").unwrap(),
                object: obj.clone(),
                value: ClaimPayload::table_grain("one row per order line", None).unwrap(),
                source: ClaimOrigin::UserExplicit,
                status: ClaimStatus::Confirmed,
            },
        ],
        conflicts: vec![ContractConflict {
            kind: "table_grain",
            claim_ids: vec![
                ClaimId::parse("c-grain0001").unwrap(),
                ClaimId::parse("c-grain0002").unwrap(),
            ],
        }],
        truncated: false,
        incomplete: false,
    };
    let name_of =
        std::collections::HashMap::from([(identity.as_str().to_string(), "analytics".into())]);
    let body = super::render::render_body(std::slice::from_ref(&contract), &name_of);
    // Both disputed claims carry the dispute marker.
    assert_eq!(
        body.matches("[disputed] ").count(),
        2,
        "both conflicting grains are disputed: {body}"
    );
    // Neither disputed claim carries the confirmed marker — the dispute
    // suppresses it, so neither reads as a binding instruction.
    assert!(
        !body.contains("[confirmed] "),
        "a disputed confirmed claim must not carry the confirmed marker: {body}"
    );
}
