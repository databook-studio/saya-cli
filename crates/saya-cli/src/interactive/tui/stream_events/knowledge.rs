//! Memory/knowledge receipt lines: supplied claims, overridden findings,
//! learning skips, and learned facts. Each shares its shaper with the
//! headless path so the wording lives in one place; all trail the answer.

use super::{BlockKind, Transcript};
use saya_agent::AgentEvent;

// What memory supplied, shown before the answer streams. The shared
// shaper centralizes the wording; an empty
// result (Ran-and-found-nothing) is silence — push nothing.
pub(crate) fn push_knowledge_supplied(
    transcript: &mut Transcript,
    outcome: saya_agent::KnowledgeOutcome,
    contracts: &[saya_agent::SuppliedContractDto],
    dropped_by_bounds: usize,
) {
    let text = crate::render::knowledge_supplied_text(outcome, contracts, dropped_by_bounds);
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
pub(crate) fn push_knowledge_overridden(
    transcript: &mut Transcript,
    findings: &[saya_agent::OverrideFindingDto],
) {
    let text = crate::render::knowledge_overridden_text(findings);
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
pub(crate) fn push_learning_skipped(
    transcript: &mut Transcript,
    reason: saya_agent::LearningSkipReason,
) {
    let text = crate::render::learning_skipped_text(reason);
    if !text.is_empty() {
        transcript.push(BlockKind::System, text.trim_end_matches('\n'));
    }
}

// The extraction circuit breaker tripped this turn (owner decision 3).
// Trails the answer, immediately after the turn's own
// `push_learning_skipped` block, so the two read together. Shares the
// shaper with the headless path; the line is never empty for a known
// model/miss pair, so the block always pushes.
pub(crate) fn push_learning_disabled(transcript: &mut Transcript, model: &str, misses: u32) {
    let text = crate::render::learning_disabled_text(model, misses);
    if !text.is_empty() {
        transcript.push(BlockKind::System, text.trim_end_matches('\n'));
    }
}

// One fact learned this turn. Trails the answer — the runtime emits it
// after the loop — so it lands below the assistant text, where "and I
// kept this" belongs. Shares the shaper with the headless path; an
// undescribable claim is silence, never a raw token.
pub(crate) fn push_knowledge_proposed(
    transcript: &mut Transcript,
    claim: &saya_agent::ProposedClaimDto,
) {
    let text = crate::render::knowledge_learned_text(claim);
    if !text.is_empty() {
        transcript.push(BlockKind::System, text.trim_end_matches('\n'));
    }
}

/// Whether the event is a memory/knowledge receipt.
#[allow(dead_code)]
pub(crate) fn is_knowledge_event(event: &AgentEvent) -> bool {
    matches!(
        event,
        AgentEvent::KnowledgeSupplied { .. }
            | AgentEvent::KnowledgeOverridden { .. }
            | AgentEvent::KnowledgeLearningSkipped { .. }
            | AgentEvent::KnowledgeLearningDisabled { .. }
            | AgentEvent::KnowledgeProposed { .. }
    )
}

/// Dispatches one memory/knowledge event. Returns true when handled.
pub(crate) fn apply_knowledge_event(transcript: &mut Transcript, event: AgentEvent) -> bool {
    match event {
        AgentEvent::KnowledgeSupplied {
            outcome,
            contracts,
            dropped_by_bounds,
        } => {
            push_knowledge_supplied(transcript, outcome, &contracts, dropped_by_bounds);
            true
        }
        AgentEvent::KnowledgeOverridden { findings } => {
            push_knowledge_overridden(transcript, &findings);
            true
        }
        AgentEvent::KnowledgeLearningSkipped { reason } => {
            push_learning_skipped(transcript, reason);
            true
        }
        AgentEvent::KnowledgeLearningDisabled { model, misses } => {
            push_learning_disabled(transcript, &model, misses);
            true
        }
        AgentEvent::KnowledgeProposed { claim } => {
            push_knowledge_proposed(transcript, &claim);
            true
        }
        _ => false,
    }
}
