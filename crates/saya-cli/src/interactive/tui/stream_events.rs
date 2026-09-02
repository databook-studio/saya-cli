//! Maps streamed agent events onto transcript blocks.

use super::table;
use super::transcript::{BlockKind, Transcript};
use saya_agent::AgentEvent;

/// Applies one streamed agent event to the transcript.
///
/// `show_thinking` gates whether the model's chain-of-thought reaches the
/// transcript: off by default, so a user who did not ask for it never sees it.
/// When on, reasoning is pushed as a dimmed `Thinking` block — visually
/// subordinate to the answer, never mistakable for it. Either way reasoning is
/// in-memory only and never persisted.
pub(crate) fn apply_event(transcript: &mut Transcript, event: AgentEvent, show_thinking: bool) {
    match event {
        AgentEvent::AssistantText { text } => {
            if !matches!(
                transcript.blocks().last().map(|b| b.kind),
                Some(BlockKind::Assistant)
            ) {
                // first chunk of the answer: separate it from the tool/SQL lines above
                if transcript
                    .blocks()
                    .last()
                    .is_some_and(|b| !b.text.is_empty())
                {
                    transcript.push(BlockKind::System, String::new());
                }
            }
            transcript.append_delta(BlockKind::Assistant, &text);
        }
        AgentEvent::ToolRequested { name, arguments } => {
            if let Some(call) = crate::agent::tools::sql_tool_call(&name, &arguments) {
                let header = match &call.target {
                    Some(t) => format!("SQL · {t}"),
                    None => "SQL".to_string(),
                };
                let body = call
                    .sql
                    .lines()
                    .map(|l| format!("  {l}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                let text = format!("{header}\n{body}");
                transcript.push(BlockKind::Tool, text);
            } else {
                let line = match crate::agent::tools::tool_call_detail(&name, &arguments) {
                    Some(detail) => format!("→ {name}: {detail}"),
                    None => format!("→ {name}"),
                };
                transcript.push(BlockKind::Tool, line);
            }
        }
        AgentEvent::ToolCompleted { name, summary } => {
            transcript.push(BlockKind::Tool, format!("✓ {name}: {summary}"))
        }
        AgentEvent::ToolDenied { name, reason } => {
            transcript.push(BlockKind::System, format!("✗ {name} denied: {reason}"))
        }
        // What memory supplied, shown before the answer streams. The shared
        // shaper centralizes the wording; an empty
        // result (Ran-and-found-nothing) is silence — push nothing.
        AgentEvent::KnowledgeSupplied {
            outcome,
            contracts,
            dropped_by_bounds,
        } => {
            let text =
                crate::render::knowledge_supplied_text(outcome, &contracts, dropped_by_bounds);
            if !text.is_empty() {
                // Strip the trailing newline: the transcript stores line text
                // without a delimiter and re-wraps per line; a trailing '\n' would
                // push a blank line into the block.
                transcript.push(BlockKind::System, text.trim_end_matches('\n'));
            }
        }
        // A confirmed claim the turn's SQL contradicted. Trails the
        // answer — emitted after the loop — so a System block pushed here lands
        // below the assistant text, where a "the SQL contradicted a confirmed
        // claim" notice belongs. The shared shaper centralizes the wording; an
        // empty finding set is silence (the runtime emits nothing, but this
        // guards a directly-constructed event too).
        AgentEvent::KnowledgeOverridden { findings } => {
            let text = crate::render::knowledge_overridden_text(&findings);
            if !text.is_empty() {
                transcript.push(BlockKind::System, text.trim_end_matches('\n'));
            }
        }
        // Extraction timed out or errored after the turn succeeded (spec
        // packet-54). Trails the answer — emitted after the loop — so a System
        // block pushed here lands below the assistant text, where "and I did
        // not learn from this turn" belongs. Shares the shaper with the
        // headless path so the wording lives in one place; the line is never
        // empty for a known reason, so the block always pushes.
        AgentEvent::KnowledgeLearningSkipped { reason } => {
            let text = crate::render::learning_skipped_text(reason);
            if !text.is_empty() {
                transcript.push(BlockKind::System, text.trim_end_matches('\n'));
            }
        }
        // One fact learned this turn. Trails the answer — the runtime emits it
        // after the loop — so it lands below the assistant text, where "and I
        // kept this" belongs. Shares the shaper with the headless path; an
        // undescribable claim is silence, never a raw token.
        AgentEvent::KnowledgeProposed { claim } => {
            let text = crate::render::knowledge_learned_text(&claim);
            if !text.is_empty() {
                transcript.push(BlockKind::System, text.trim_end_matches('\n'));
            }
        }
        // The model's chain-of-thought. Shown only when the user opted in; otherwise
        // accepted and dropped, so the event never reaches the catch-all and never
        // renders as an error. When shown it lands as a dimmed `Thinking` block,
        // separate from the assistant answer and visually subordinate to it. Reasoning
        // is in-memory only: the transcript is never serialized, and the persisted
        // session types carry role + content only, so holding it here cannot reach a
        // session file regardless of the display toggle.
        AgentEvent::ReasoningText { text } => {
            if show_thinking && !text.is_empty() {
                transcript.push(BlockKind::Thinking, text);
            }
        }
        AgentEvent::Complete => {
            transcript.reformat_last(BlockKind::Assistant, table::format_markdown_tables);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use saya_agent::{
        KnowledgeOutcome, LearningSkipReason, OverrideFindingDto, SuppliedClaimDto,
        SuppliedContractDto,
    };
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
        // the batch-approve slice folded-in: the header points at /queue, the action the learn
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
        let fake_identity =
            "sha256:deadbeefcafef00d1234567890abcdef1234567890abcdef1234567890abcdef";
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

    /// KnowledgeOverridden pushes a System block whose text names the referenced
    /// column and the specified value. Trails the answer — the
    /// block lands below the assistant text in the transcript.
    #[test]
    fn knowledge_overridden_pushes_a_system_block_naming_the_finding() {
        let mut t = Transcript::new();
        apply_event(
            &mut t,
            AgentEvent::knowledge_overridden(vec![OverrideFindingDto {
                claim_id: ClaimId::parse("c-rental-time").unwrap(),
                kind: "default_time_column".into(),
                claimed_value: "return_date".into(),
                observed_columns: vec!["rental_date".into()],
            }]),
            false,
        );
        let block = last_block_text(&t).expect("a block was pushed");
        assert!(
            block.contains("memory overridden · 1 finding"),
            "header: {block}"
        );
        assert!(
            block.contains("referenced rental_date"),
            "names the referenced column: {block}"
        );
        assert!(
            block.contains("where you specified return_date"),
            "names the specified value: {block}"
        );
        // The wording constraint: no causal "used" about the time column.
        assert!(
            !block.contains("used"),
            "the TUI block must not assert a causal 'used': {block}"
        );
        assert_eq!(t.blocks().last().unwrap().kind, BlockKind::System);
    }

    /// An empty finding set pushes nothing — silence.
    #[test]
    fn an_empty_knowledge_overridden_event_pushes_nothing() {
        let mut t = Transcript::new();
        apply_event(&mut t, AgentEvent::knowledge_overridden(Vec::new()), false);
        assert!(
            t.blocks().is_empty(),
            "no findings → no block: {:?}",
            t.blocks()
        );
    }

    /// KnowledgeLearningSkipped pushes a System block whose text names the skip
    /// reason (packet-54 decision 4 — the TUI renders it, it does not fall
    /// through to the catch-all that would drop it). Trails the answer.
    #[test]
    fn knowledge_learning_skipped_pushes_a_system_block_naming_the_reason() {
        let mut t = Transcript::new();
        apply_event(
            &mut t,
            AgentEvent::knowledge_learning_skipped(LearningSkipReason::TimedOut),
            false,
        );
        let block = last_block_text(&t).expect("a block was pushed");
        assert!(
            block.contains("memory not recorded · extraction timed out"),
            "TUI block names the timeout: {block}"
        );
        assert_eq!(t.blocks().last().unwrap().kind, BlockKind::System);

        let mut t = Transcript::new();
        apply_event(
            &mut t,
            AgentEvent::knowledge_learning_skipped(LearningSkipReason::Failed),
            false,
        );
        let block = last_block_text(&t).expect("a block was pushed");
        assert!(
            block.contains("memory not recorded · extraction failed"),
            "TUI block names the failure: {block}"
        );
    }

    /// The model's chain-of-thought reaches the transcript only when the user
    /// opted in. Off by default, so a user who did not ask for it never sees it;
    /// on, it lands as a `Thinking` block separate from the assistant answer.
    /// Asserts on the transcript state, the same seam the other `apply_event`
    /// tests use.
    #[test]
    fn reasoning_text_is_silent_when_thinking_is_off() {
        let mut t = Transcript::new();
        apply_event(
            &mut t,
            AgentEvent::reasoning_text("I considered the time column"),
            false,
        );
        assert!(
            t.blocks().is_empty(),
            "reasoning must not reach the transcript when thinking is off: {:?}",
            t.blocks()
        );
        // It stays silent even when an answer has already streamed — it does
        // not push a block above, below, or between assistant blocks.
        apply_event(&mut t, AgentEvent::assistant_text("the answer"), false);
        apply_event(
            &mut t,
            AgentEvent::reasoning_text("more thinking mid-turn"),
            false,
        );
        assert_eq!(
            t.blocks().len(),
            1,
            "only the assistant block should be present: {:?}",
            t.blocks()
        );
        assert_eq!(t.blocks()[0].kind, BlockKind::Assistant);
        assert!(
            !t.blocks()[0].text.contains("thinking"),
            "reasoning must not be folded into the assistant block: {:?}",
            t.blocks()[0].text
        );
    }

    /// When thinking is on, a `ReasoningText` event pushes a `Thinking` block
    /// carrying the chain-of-thought — separate from the assistant answer, so
    /// it cannot be mistaken for it. Empty reasoning pushes nothing either way.
    #[test]
    fn reasoning_text_pushes_a_thinking_block_when_thinking_is_on() {
        let mut t = Transcript::new();
        apply_event(
            &mut t,
            AgentEvent::reasoning_text("I considered the time column"),
            true,
        );
        assert_eq!(t.blocks().len(), 1, "one thinking block is pushed");
        assert_eq!(t.blocks()[0].kind, BlockKind::Thinking);
        assert_eq!(t.blocks()[0].text, "I considered the time column");

        // Empty reasoning pushes nothing, even with thinking on.
        apply_event(&mut t, AgentEvent::reasoning_text(""), true);
        assert_eq!(t.blocks().len(), 1, "empty reasoning pushes no block");
    }

    /// Displaying the chain-of-thought must not open a path for it to reach a
    /// persisted session. The reasoning restates row values and column
    /// contents in prose, and session files are redacted against a different
    /// threat, so the display toggle and the persistence boundary stay
    /// independent: turning the former on must not weaken the latter.
    ///
    /// The reasoning is asserted to be on screen first. Without that, the
    /// absence checks below would pass on a build that leaked, because a
    /// string never introduced is trivially absent.
    #[test]
    fn reasoning_shown_on_screen_stays_out_of_the_persisted_session() {
        use crate::interactive::session_state::SessionState;

        let reasoning = "the secret chain-of-thought about row values 9f3a";
        let mut transcript = Transcript::default();
        apply_event(&mut transcript, AgentEvent::reasoning_text(reasoning), true);
        apply_event(
            &mut transcript,
            AgentEvent::assistant_text("the answer is 42"),
            true,
        );

        assert!(
            transcript
                .blocks()
                .iter()
                .any(|b| b.kind == BlockKind::Thinking && b.text.contains(reasoning)),
            "the reasoning must be on screen for this test to prove anything"
        );

        let mut session =
            SessionState::new("s1", Some(String::from("analytics")), String::from("m"));
        session.show_thinking = true;
        session.record_turn("what is the answer", "the answer is 42", false, Vec::new());

        let json = serde_json::to_string(&session).expect("serializes");
        assert!(
            !json.contains(reasoning),
            "reasoning on screen leaked into the persisted session: {json}"
        );

        let replayed = session.provider_history();
        assert!(
            replayed.iter().all(|m| !m.content.contains(reasoning)),
            "reasoning on screen leaked into replayed history: {replayed:?}"
        );

        let redacted = session.redacted();
        let redacted_json = serde_json::to_string(&redacted).expect("serializes");
        assert!(
            !redacted_json.contains(reasoning),
            "reasoning on screen leaked into the redacted session: {redacted_json}"
        );
    }
}
