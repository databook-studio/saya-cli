//! Rendering of contract views: text shape (insta), JSON/NDJSON round-trip, and
//! the structural guarantee that no opaque profile identity can leak.

use saya_cli::{
    ContractClaimView, ContractConflictView, ContractView, RenderFormat, TerminalEvent,
    render_event,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A two-claim contract on `analytics.public.orders`, one current claim and one
/// referencing a column whose state we vary per test.
fn orders_view(schema_state: &str, truncated: bool) -> ContractView {
    ContractView {
        profile: "warehouse".into(),
        object: "analytics.public.orders".into(),
        schema_state: schema_state.into(),
        claims: vec![
            ContractClaimView {
                claim_id: "c-1a2b3c4d5e6".into(),
                kind: "default_time_column".into(),
                origin: "user_explicit".into(),
                status: "confirmed".into(),
                value: "created_at".into(),
                column: Some("created_at".into()),
                reason: None,
            },
            ContractClaimView {
                claim_id: "c-3c4d5e6f7a8b".into(),
                kind: "table_alias".into(),
                origin: "user_explicit".into(),
                status: "confirmed".into(),
                value: "customers".into(),
                column: None,
                reason: None,
            },
        ],
        conflicts: Vec::new(),
        truncated,
    }
}

/// `contracts show` renders a directive claim's reason beneath the claim line,
/// so a user auditing a claim sees why it exists (spec: claim-reasons). `list`
/// does not — it is a one-line-per-claim inventory and a reason is a sentence.
#[test]
fn text_contract_show_renders_a_reason_under_the_claim() {
    let mut contract = orders_view("current", false);
    contract.claims[0].reason = Some("an order only completes when it ships".into());
    let event = TerminalEvent::ContractShow { contract };
    let stdout = render_event(&event, RenderFormat::Text).stdout;
    // The claim line is intact.
    assert!(
        stdout.contains("default_time_column  confirmed  user_explicit  created_at"),
        "claim line intact: {stdout}"
    );
    // The reason follows, indented under the claim.
    assert!(
        stdout.contains("    because: an order only completes when it ships"),
        "show renders the reason: {stdout}"
    );
}

/// `contracts list` does not render the reason — it is a one-line inventory.
#[test]
fn text_contract_list_does_not_render_a_reason() {
    let mut contract = orders_view("current", false);
    contract.claims[0].reason = Some("an order only completes when it ships".into());
    let event = TerminalEvent::ContractList {
        contracts: vec![contract],
    };
    let stdout = render_event(&event, RenderFormat::Text).stdout;
    assert!(
        stdout.contains("default_time_column  confirmed  user_explicit  created_at"),
        "claim line present: {stdout}"
    );
    assert!(
        !stdout.contains("because:"),
        "list must not render the reason: {stdout}"
    );
}

#[test]
fn text_contract_list_with_current_and_stale() {
    let event = TerminalEvent::ContractList {
        contracts: vec![
            orders_view("current", false),
            ContractView {
                profile: "warehouse".into(),
                object: "analytics.public.returns".into(),
                schema_state: "stale".into(),
                claims: vec![ContractClaimView {
                    claim_id: "c-9z9z9z9z9z9z".into(),
                    kind: "column_role".into(),
                    origin: "schema_observed".into(),
                    status: "stale".into(),
                    value: "identifier".into(),
                    column: Some("return_id".into()),
                    reason: None,
                }],
                conflicts: Vec::new(),
                truncated: false,
            },
        ],
    };
    insta::assert_snapshot!(render_event(&event, RenderFormat::Text).stdout);
}

#[test]
fn text_contract_show_with_conflict_and_truncated() {
    let mut contract = orders_view("needs_review", true);
    contract.conflicts = vec![ContractConflictView {
        kind: "table_grain".into(),
        claim_ids: vec!["c-3c4d5e6f7a8b".into(), "c-5e6f7a8b9c0d".into()],
    }];
    let event = TerminalEvent::ContractShow { contract };
    insta::assert_snapshot!(render_event(&event, RenderFormat::Text).stdout);
}

#[test]
fn text_contract_changed_remembered() {
    let event = TerminalEvent::ContractChanged {
        claim_id: "c-1a2b3c4d5e6".into(),
        action: "remembered".into(),
        status: "confirmed".into(),
    };
    insta::assert_snapshot!(render_event(&event, RenderFormat::Text).stdout);
}

#[test]
fn text_contract_changed_confirmed() {
    let event = TerminalEvent::ContractChanged {
        claim_id: "c-1a2b3c4d5e6".into(),
        action: "confirmed".into(),
        status: "confirmed".into(),
    };
    insta::assert_snapshot!(render_event(&event, RenderFormat::Text).stdout);
}

#[test]
fn text_contract_changed_rejected() {
    let event = TerminalEvent::ContractChanged {
        claim_id: "c-1a2b3c4d5e6".into(),
        action: "rejected".into(),
        status: "rejected".into(),
    };
    insta::assert_snapshot!(render_event(&event, RenderFormat::Text).stdout);
}

#[test]
fn text_contract_changed_forgotten() {
    let event = TerminalEvent::ContractChanged {
        claim_id: "c-1a2b3c4d5e6".into(),
        action: "forgotten".into(),
        status: "forgotten".into(),
    };
    insta::assert_snapshot!(render_event(&event, RenderFormat::Text).stdout);
}

/// A duplicate of an already-forgotten claim must read as "previously forgotten",
/// not as a fresh success — the user has to know the claim is gone.
#[test]
fn text_contract_changed_duplicate_of_forgotten() {
    let event = TerminalEvent::ContractChanged {
        claim_id: "c-1a2b3c4d5e6".into(),
        action: "duplicate".into(),
        status: "forgotten".into(),
    };
    insta::assert_snapshot!(render_event(&event, RenderFormat::Text).stdout);
}

#[test]
fn text_contract_changed_duplicate_of_confirmed() {
    let event = TerminalEvent::ContractChanged {
        claim_id: "c-1a2b3c4d5e6".into(),
        action: "duplicate".into(),
        status: "confirmed".into(),
    };
    insta::assert_snapshot!(render_event(&event, RenderFormat::Text).stdout);
}

#[test]
fn contract_view_json_round_trip() {
    let view = orders_view("current", true);
    let json = serde_json::to_string(&view).expect("serializable");
    let back: ContractView = serde_json::from_str(&json).expect("deserializable");
    assert_eq!(view, back);
}

#[test]
fn contract_view_ndjson_event_round_trip() {
    let contract = orders_view("stale", false);
    let event = TerminalEvent::ContractShow {
        contract: contract.clone(),
    };
    let rendered = render_event(&event, RenderFormat::Ndjson);
    // NDJSON is one JSON object per line; the event serializes to a single line.
    let line = rendered.stdout.trim_end_matches('\n');
    let value: Value = serde_json::from_str(line).expect("ndjson line is JSON");
    // The event tags as "contract_show" (snake_case) and nests the contract.
    assert_eq!(value["event"], "contract_show");
    let back: ContractView =
        serde_json::from_value(value["contract"].clone()).expect("nested contract deserializes");
    assert_eq!(contract, back);
}

#[test]
fn empty_contract_list_is_one_plain_line_not_an_error() {
    let event = TerminalEvent::ContractList { contracts: vec![] };
    let rendered = render_event(&event, RenderFormat::Text);
    assert!(
        rendered.stderr.is_empty(),
        "empty list is not an error: {rendered:?}"
    );
    let stdout = rendered.stdout.trim_end_matches('\n');
    assert!(!stdout.is_empty(), "empty list still says something");
    assert!(
        !stdout.contains('\n'),
        "empty list is a single plain line, got:\n{stdout}"
    );
}

/// The leak this prevents: an opaque profile identity (a hash over host, database,
/// user and scope path) being carried in a serialized contract payload. The
/// `ContractView` type shape must exclude any identity-shaped key — there is no
/// field for it, and the test proves that by construction: the set of keys a
/// fully-populated `ContractView` serializes to is exactly the expected set, with
/// no `identity` / `profile_identity` / `id` key present.
///
/// (Spec §4 test 5 described a runtime "seed every field with `p-<64 hex>` and
/// render" test, then concluded the real assertion is the type-shape one. The
/// runtime half is dropped here — it cannot prove what it claims, since the
/// profile name is legitimately a string and an identity-shaped value in it would
/// be indistinguishable from a name. Only the structural assertion can catch the
/// leak.)
#[test]
fn contract_view_serialized_keys_exclude_opaque_profile_identity() {
    // A fully-populated view so no `skip_serializing_if` field is elided:
    // `truncated: true`, a conflict present, and a claim with `column` and
    // `reason` present (the two optional fields, both `Some`).
    let view = ContractView {
        profile: "warehouse".into(),
        object: "analytics.public.orders".into(),
        schema_state: "current".into(),
        claims: vec![ContractClaimView {
            claim_id: "c-1".into(),
            kind: "column_role".into(),
            origin: "user_explicit".into(),
            status: "confirmed".into(),
            value: "identifier".into(),
            column: Some("id".into()),
            reason: Some("the row's stable identity".into()),
        }],
        conflicts: vec![ContractConflictView {
            kind: "table_grain".into(),
            claim_ids: vec!["c-1".into()],
        }],
        truncated: true,
    };
    let json = serde_json::to_value(&view).expect("serializable to JSON value");
    let object = json
        .as_object()
        .expect("ContractView serializes to a JSON object");
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "claims",
            "conflicts",
            "object",
            "profile",
            "schema_state",
            "truncated"
        ],
        "ContractView must carry no identity-shaped field"
    );

    // A claim's keys too — the only place an identity could hide inside a contract.
    let claim_json = serde_json::to_value(&view.claims[0]).expect("claim serializable");
    let mut claim_keys: Vec<&str> = claim_json
        .as_object()
        .expect("claim serializes to an object")
        .keys()
        .map(String::as_str)
        .collect();
    claim_keys.sort();
    assert_eq!(
        claim_keys,
        [
            "claim_id", "column", "kind", "origin", "reason", "status", "value"
        ],
        "ContractClaimView must carry no identity-shaped field"
    );

    // Compile-time shape check: the DTOs implement both directions, so the wire
    // shape is a real contract, not a one-way print.
    fn _assert_serializable<T: Serialize + for<'de> Deserialize<'de>>() {}
    _assert_serializable::<ContractView>();
    _assert_serializable::<ContractClaimView>();
    _assert_serializable::<ContractConflictView>();
}
