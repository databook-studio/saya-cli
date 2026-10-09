use super::*;

/// A conflict does not suppress the object's other, non-disputed claims: a
/// confirmed alias on the same object as two conflicting grains still appears
/// and carries no dispute marker.
#[tokio::test]
async fn conflict_does_not_suppress_non_disputed_claims() {
    use crate::contracts::{ContractClaim, ContractConflict, RetrievedContract};
    // D-3 NOTE: two confirmed grains cannot coexist in `knowledge_items`
    // (single-valued slot); see `conflicting_grains_both_appear_marked_and_kind_named`.
    // This renders a directly-constructed contract so the behaviour it guards —
    // a non-disputed claim surviving alongside disputed ones — still holds.
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
            ContractClaim {
                id: ClaimId::parse("c-alias0001").unwrap(),
                object: obj.clone(),
                value: ClaimPayload::table_alias("orders").unwrap(),
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
    // The non-disputed alias survives and reads as an established fact.
    assert!(
        body.contains("table_alias  orders"),
        "non-disputed alias survives the conflict: {body}"
    );
    // Exactly the two grains are disputed — the alias is not.
    assert_eq!(
        body.matches("[disputed]").count(),
        2,
        "only the conflicting grains are marked disputed: {body}"
    );
}

/// Conflict and candidate marking compose: under `include-candidates`, a
/// contract can carry both a disputed confirmed pair and an admitted candidate,
/// and both markers appear in the same block. A candidate
/// can never itself be disputed — `is_recallable` is `Confirmed` only, so
/// conflict detection never names a candidate id (SPEC REVIEW) — but the two
/// in-band markers coexist on different lines.
#[tokio::test]
async fn conflict_and_candidate_markers_compose_in_one_block() {
    use crate::contracts::{ContractClaim, ContractConflict, RetrievedContract};
    // D-3 NOTE: two confirmed grains cannot coexist in `knowledge_items`
    // (single-valued slot); see `conflicting_grains_both_appear_marked_and_kind_named`.
    // This renders a directly-constructed contract so the behaviour it guards —
    // a candidate marker composing with disputed markers in one stanza — holds.
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
            ContractClaim {
                id: ClaimId::parse("c-time0001").unwrap(),
                object: obj.clone(),
                value: ClaimPayload::default_time_column("created_at", None).unwrap(),
                source: ClaimOrigin::AssistantInferred,
                status: ClaimStatus::Candidate,
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
    // The candidate is admitted and marked unconfirmed.
    assert!(body.contains("created_at"), "candidate reaches the body");
    assert!(
        body.contains("[candidate — unconfirmed]"),
        "candidate marker appears: {body}"
    );
    // The confirmed grains are disputed.
    assert_eq!(
        body.matches("[disputed]").count(),
        2,
        "both grains disputed alongside the candidate: {body}"
    );
}

/// The opaque profile identity still appears nowhere in a conflict block: the
/// dispute summary names the kind and count, never the identity.
#[tokio::test]
async fn opaque_identity_appears_nowhere_in_conflict_block() {
    use crate::contracts::{ContractClaim, ContractConflict, RetrievedContract};
    // D-3 NOTE: two confirmed grains cannot coexist in `knowledge_items`; see
    // `conflicting_grains_both_appear_marked_and_kind_named`. Render a
    // directly-constructed conflict contract so the identity-leak guard the
    // test carries (the dispute summary names kind/count, never the identity)
    // still runs.
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
        !body.contains(identity.as_str()),
        "opaque identity leaked into conflict block: {body}"
    );
    assert!(body.contains("analytics"), "profile name appears instead");
}
