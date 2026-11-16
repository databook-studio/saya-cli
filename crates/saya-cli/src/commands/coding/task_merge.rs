//! Owner-side direct-mode summary of one finished helper. Direct mode only.
//! The `helpers` tool calls it; it carries no authority.
//!
//! Helpers work in the root's own working checkout W, so there is no
//! per-helper checkout, no merge step and no `records/merges.jsonl`: the
//! helper's own writes are already in the checkout the operator reviews. What
//! survives here is the redacted, capped, authority-free summary the
//! coordinator reads. The value types and the store lookups live in
//! `task_merge_record`.

use super::task_merge_record::{ChildSummary, cap_summary};
use super::{CodingError, NativeCodingJob};
use saya_types::AgentId;

impl NativeCodingJob {
    /// A redacted, capped summary of one finished helper, carrying no
    /// authority. A helper writes W directly, so there is no per-helper delta
    /// to attribute: the summary is the child's own answer, capped and
    /// redacted.
    pub(crate) async fn child_summary(&self, agent: &AgentId) -> Result<ChildSummary, CodingError> {
        let completion = self.find_completion(agent).await?;
        let (summary, summary_truncated) = cap_summary(completion.result.answer());
        Ok(ChildSummary {
            agent: agent.clone(),
            outcome: completion.result.outcome(),
            failure_class: completion.result.failure_class(),
            summary,
            summary_truncated,
            assignment_added: super::assignment_added::assignment_added_lines(
                completion.result.answer(),
            ),
            authority: "none",
        })
    }
}
