//! Tests for the KnowledgeSupplied receipt line, driven through `super::apply_event` (moved verbatim
//! from the inline `tests` module in `stream_events/apply.rs`).

use super::{BlockKind, Transcript, apply_event};
use saya_agent::AgentEvent;
use saya_agent::{KnowledgeOutcome, SuppliedClaimDto, SuppliedContractDto};
use saya_types::{ClaimId, ClaimStatus};

fn dto_claim(id: &str, kind: &str, value: &str, status: ClaimStatus) -> SuppliedClaimDto {
    SuppliedClaimDto {
        claim_id: ClaimId::parse(id).unwrap(),
        kind: kind.into(),
        value: value.into(),
        column: None,
        status,
    }
}

fn dto_contract(claims: Vec<SuppliedClaimDto>) -> SuppliedContractDto {
    SuppliedContractDto {
        profile: "analytics".into(),
        object: "catalog.public.orders".into(),
        schema_state: "current".into(),
        claims,
    }
}

fn last_block_text(transcript: &Transcript) -> Option<&str> {
    transcript.blocks().last().map(|b| b.text.as_str())
}

/// KnowledgeSupplied pushes a System block whose text names the supplied
/// claims. Asserts on the rendered transcript, not state.
#[test]
fn knowledge_supplied_pushes_a_system_block_with_the_claims() {
    let mut t = Transcript::new();
    apply_event(
        &mut t,
        AgentEvent::knowledge_supplied(
            KnowledgeOutcome::Ran {
                store_unavailable: false,
            },
            vec![dto_contract(vec![
                dto_claim("c-1", "table_alias", "orders", ClaimStatus::Confirmed),
                dto_claim(
                    "c-2",
                    "default_time_column",
                    "created_at",
                    ClaimStatus::Candidate,
                ),
            ])],
            0,
        ),
        false,
    );
    let block = last_block_text(&t).expect("a block was pushed");
    // The header points at /queue, the action the learn
    // path already names, beside the unconfirmed count it always carried.
    assert!(
        block.starts_with("memory supplied · 2 claims (1 unconfirmed — review with /queue)"),
        "{block}"
    );
    assert!(block.contains("table_alias  orders  confirmed"), "{block}");
    // The candidate is marked on its line.
    assert!(block.contains("candidate  (unconfirmed)"), "{block}");
    // The block is a System block, not a Tool block.
    assert_eq!(t.blocks().last().unwrap().kind, BlockKind::System);
}

/// The three outcomes are distinguishable in the transcript, and
/// Ran-and-found-nothing pushes nothing (silence) (spec §5 / §4).
#[test]
fn tui_distinguishes_the_three_outcomes() {
    let mut t = Transcript::new();
    apply_event(
        &mut t,
        AgentEvent::knowledge_supplied(KnowledgeOutcome::Off, Vec::new(), 0),
        false,
    );
    assert_eq!(last_block_text(&t), Some("memory off · recall disabled"));

    let mut t = Transcript::new();
    apply_event(
        &mut t,
        AgentEvent::knowledge_supplied(KnowledgeOutcome::Skipped, Vec::new(), 0),
        false,
    );
    assert_eq!(
        last_block_text(&t),
        Some("memory skipped · not permitted to read saved claims")
    );

    // Ran-and-found-nothing: nothing is pushed (silence).
    let mut t = Transcript::new();
    apply_event(
        &mut t,
        AgentEvent::knowledge_supplied(
            KnowledgeOutcome::Ran {
                store_unavailable: false,
            },
            Vec::new(),
            0,
        ),
        false,
    );
    assert!(
        t.blocks().is_empty(),
        "Ran-nothing pushes nothing: {:?}",
        t.blocks()
    );
}

/// A non-zero dropped count is visible in the pushed block (spec §5).
#[test]
fn tui_shows_a_nonzero_dropped_count() {
    let mut t = Transcript::new();
    apply_event(
        &mut t,
        AgentEvent::knowledge_supplied(
            KnowledgeOutcome::Ran {
                store_unavailable: false,
            },
            vec![dto_contract(vec![dto_claim(
                "c-1",
                "table_alias",
                "orders",
                ClaimStatus::Confirmed,
            )])],
            30,
        ),
        false,
    );
    let block = last_block_text(&t).expect("a block was pushed");
    assert!(block.contains("· 30 more dropped by bounds"), "{block}");
}

/// No opaque profile identity reaches the transcript: the profile name
/// appears, a fabricated identity does not (spec §5 / §4).
#[test]
fn tui_does_not_leak_an_opaque_identity() {
    let fake_identity = "sha256:deadbeefcafef00d1234567890abcdef1234567890abcdef1234567890abcdef";
    let mut t = Transcript::new();
    apply_event(
        &mut t,
        AgentEvent::knowledge_supplied(
            KnowledgeOutcome::Ran {
                store_unavailable: false,
            },
            vec![dto_contract(vec![dto_claim(
                "c-1",
                "table_alias",
                "orders",
                ClaimStatus::Confirmed,
            )])],
            0,
        ),
        false,
    );
    let block = last_block_text(&t).expect("a block was pushed");
    assert!(block.contains("analytics"), "profile name appears: {block}");
    assert!(!block.contains(fake_identity), "identity leaked: {block}");
}
