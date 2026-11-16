//! Rendering of contract views: text shape (insta), JSON/NDJSON round-trip, and
//! the structural guarantee that no opaque profile identity can leak.

use saya_cli::{
    ContractClaimView, ContractConflictView, ContractQueueItemView, ContractView, RenderFormat,
    TerminalEvent, render_event,
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
        incomplete: false,
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
                incomplete: false,
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

#[test]
fn text_contract_changed_cleanup_pending_is_explicit() {
    let event = TerminalEvent::ContractChanged {
        claim_id: "c-1a2b3c4d5e6".into(),
        action: "forgotten_cleanup_pending".into(),
        status: "forgotten".into(),
    };
    assert_eq!(
        render_event(&event, RenderFormat::Text).stdout,
        "forgotten c-1a2b3c4d5e6 (cleanup pending — retry to complete)\n"
    );
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

include!("contract_render/wire_contracts.rs");
