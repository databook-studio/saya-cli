//! Text shaping for [`AgentEvent::KnowledgeSupplied`].
//!
//! The wording here is a correctness constraint, not style: the line names
//! **claims** (never *facts*) and says **supplied** (never *applied* or *used*).
//! Recall injects context; the model may ignore it — a confirmed claim being
//! supplied does not mean the generated SQL honoured it (we have measured that
//! it frequently does not). A causal verb would overstate what we know, in the
//! one feature whose pitch is that it does not overstate.
//!
//! This shaper is shared by the text/CLI path ([`super::TerminalEvent`] →
//! [`super::render_event`]) and the TUI path ([`crate::interactive::tui`]
//! `apply_event`) so the wording lives in one place. Both adapters lead — the
//! line renders when the event arrives, before the answer streams (spec §6).

use saya_agent::{
    KnowledgeOutcome, LearningSkipReason, OverrideFindingDto, SuppliedClaimDto, SuppliedContractDto,
};
use saya_types::ClaimStatus;

include!("render_memory/supplied.rs");
include!("render_memory/overrides.rs");
include!("render_memory/learning.rs");
#[cfg(test)]
mod tests {
    use super::*;
    include!("render_memory/tests/setup.rs");
    include!("render_memory/tests/supplied_cases.rs");
    include!("render_memory/tests/supply_edges.rs");
    include!("render_memory/tests/override_cases.rs");
    include!("render_memory/tests/learning_cases.rs");
}
