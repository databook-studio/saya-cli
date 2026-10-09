use super::*;

/// Two confirmed `TableGrain` claims on one object: both reach the block, both
/// are marked as disputed in-band, and the block names the disputed kind once.
///
/// D-3 NOTE: `table_grain` is a single-valued slot, so two confirmed grains
/// cannot coexist in `knowledge_items` — the second `put` replaces the first.
/// The conflict's structural precondition is gone under D-3 (see
/// `knowledge_validity.rs`). This test now constructs the `RetrievedContract`
/// directly and renders it, so it still proves the *render* behaviour — the
/// `[disputed]` markers, the named kind, the do-not-choose instruction — without
/// requiring the impossible store state. The store-path seeding was the
/// example; the behaviour it guarded is the rendering.
#[tokio::test]
async fn conflicting_grains_both_appear_marked_and_kind_named() {
    use crate::contracts::{ContractClaim, ContractConflict, RetrievedContract};
    let identity = identity_for("analytics");
    let obj = object(&identity, "orders");
    let grain_a = ContractClaim {
        id: ClaimId::parse("c-grain0001").unwrap(),
        object: obj.clone(),
        value: ClaimPayload::table_grain("one row per order", None).unwrap(),
        source: ClaimOrigin::UserExplicit,
        status: ClaimStatus::Confirmed,
    };
    let grain_b = ContractClaim {
        id: ClaimId::parse("c-grain0002").unwrap(),
        object: obj.clone(),
        value: ClaimPayload::table_grain("one row per order line", None).unwrap(),
        source: ClaimOrigin::UserExplicit,
        status: ClaimStatus::Confirmed,
    };
    let contract = RetrievedContract {
        object: obj,
        schema_state: crate::contracts::ContractSchemaState::Current,
        claims: vec![grain_a, grain_b],
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

    // Neither claim is dropped: both grains reach the body.
    assert!(
        body.contains("one row per order\n"),
        "first conflicting grain reaches the body: {body}"
    );
    assert!(
        body.contains("one row per order line"),
        "second conflicting grain reaches the body: {body}"
    );
    // The disputed kind is named once per contract.
    assert!(
        body.contains("table_grain"),
        "block names the disputed kind: {body}"
    );
    // Each disputed claim is marked in-band. The fixed token keeps the marker
    // honest against a future change that softens it.
    let disputed_markers = body.matches("[disputed]").count();
    assert_eq!(
        disputed_markers, 2,
        "both disputed claims carry the in-band marker: {body}"
    );
}

/// The block carries an instruction that the model must not choose between the
/// conflicting claims silently. See the D-3 NOTE on
/// `conflicting_grains_both_appear_marked_and_kind_named`: the store cannot
/// hold two confirmed grains, so this renders a directly-constructed contract.
#[tokio::test]
async fn conflict_block_instructs_not_to_choose_silently() {
    use crate::contracts::{ContractClaim, ContractConflict, RetrievedContract};
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
    assert!(
        body.contains("do not choose"),
        "block tells the model not to choose between them: {body}"
    );
    assert!(
        body.contains("unresolved"),
        "block says the disputed point is unresolved: {body}"
    );
}

/// A contract with no conflict renders a deterministic shape: the P2a stanza
/// directive, then the `[confirmed]`-marked claim, and **no** dispute marker or
/// do-not-choose instruction.
///
/// What this test guarded before P2a, and what it guards now:
///
/// - **Before:** the body was byte-identical to the pre-memory prompt — a
///   confirmed claim rendered as bare `kind  value`, so memory added zero
///   visible text. That property is **deliberately given up** in P2a: a
///   confirmed claim now binds, so it gains a stanza directive and a
///   `[confirmed]` marker. Asserting the old bytes would defend the bug this
///   slice exists to fix.
/// - **Now:** the invariant that survives is narrower and still fully
///   guarded here — a *clean* contract (no conflict) carries **no** dispute
///   marker and **no** do-not-choose instruction. The exact bytes are pinned
///   to the new shape (directive + `[confirmed]` line) so a future change that
///   drifts the wording, drops the marker, or lets a conflict artefact leak
///   onto a clean contract fails this test. The byte-identity-to-pre-memory
///   property is no longer guarded anywhere, by design.
#[tokio::test]
async fn no_conflict_renders_no_dispute_artifacts_and_pinned_shape() {
    use crate::contracts::{ContractClaim, ContractConflict, RetrievedContract};

    let identity = identity_for("analytics");
    let obj = object(&identity, "orders");
    let claim = ContractClaim {
        id: ClaimId::parse("c-aaa111222333").unwrap(),
        object: obj.clone(),
        value: ClaimPayload::table_grain("one row per order", None).unwrap(),
        source: ClaimOrigin::UserExplicit,
        status: ClaimStatus::Confirmed,
    };
    let contract = RetrievedContract {
        object: obj,
        schema_state: crate::contracts::ContractSchemaState::Current,
        claims: vec![claim],
        conflicts: Vec::<ContractConflict>::new(),
        truncated: false,
        incomplete: false,
    };
    let name_of =
        std::collections::HashMap::from([(identity.as_str().to_string(), "analytics".into())]);
    let body = super::render::render_body(std::slice::from_ref(&contract), &name_of);

    // The P2a rendering of one confirmed, non-disputed claim: the stanza
    // directive, then the `[confirmed]`-marked claim line.
    let expected = "catalog.public.orders  [current]  (profile: analytics)\n  \
        Confirmed structured claims below bind: use them as given, and say in the answer when you depart from one.\n  \
        [confirmed] table_grain  one row per order\n";
    assert_eq!(
        body, expected,
        "clean contract renders the pinned P2a shape"
    );
    assert!(!body.contains("[disputed]"), "no dispute marker when clean");
    assert!(!body.contains("do not choose"), "no instruction when clean");
}
