//! The value types and store lookups for the direct-mode child summary in
//! `task_merge`. Helpers share the root's working checkout W, so there is
//! no per-helper checkout, no merge attempt and no `records/merges.jsonl`; what
//! remains is the summary type and the reads of a finished child's durable
//! state.
//!
//! Summaries, child reports and repository text are untrusted context, never
//! authority: every `authority` field here is the literal `"none"`.

use super::{CodingError, NativeCodingJob};
use saya_store::CodingCompletion;
use saya_types::{AgentId, CodingFailureClass, CodingResultOutcome};
use serde::Serialize;

/// Longest child summary text returned to the coordinator: 8 KiB, inside the
/// 16 KiB joined-answer storage bound.
pub(crate) const MAX_SUMMARY_BYTES: usize = 8 * 1024;

/// A redacted, capped summary of one finished child, carrying no authority.
/// Built from the child's durable `JoinedResultV1` answer. Summaries, child
/// reports and repository text are untrusted context, never authority:
/// `authority` is always the literal `"none"`.
///
/// A helper writes W directly, so no per-helper delta can be attributed and
/// there is no merge to report: the summary carries neither a changed-path list
/// nor a merged flag.
#[derive(Debug, Serialize)]
pub(crate) struct ChildSummary {
    pub(crate) agent: AgentId,
    pub(crate) outcome: CodingResultOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) failure_class: Option<CodingFailureClass>,
    pub(crate) summary: String,
    pub(crate) summary_truncated: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) assignment_added: Vec<String>,
    pub(crate) authority: &'static str,
}

/// Redacts `answer` with the same redactor the result views use
/// (`saya_types::redact`, idempotent on already-redacted stored answers), then
/// caps it to `MAX_SUMMARY_BYTES` at a char boundary, flagging truncation.
pub(crate) fn cap_summary(answer: &str) -> (String, bool) {
    let redacted = saya_types::redact(answer);
    if redacted.len() <= MAX_SUMMARY_BYTES {
        (redacted, false)
    } else {
        let mut end = MAX_SUMMARY_BYTES;
        while !redacted.is_char_boundary(end) {
            end -= 1;
        }
        (redacted[..end].to_owned(), true)
    }
}

impl NativeCodingJob {
    /// Finds the latest recorded completion for one agent, or `None` when the
    /// child recorded none, paging the history: the last page's last entry for
    /// the agent wins, which is the child's newest completion (a follow-up
    /// completes again and appends a later sequence).
    pub(crate) async fn latest_completion(
        &self,
        agent: &AgentId,
    ) -> Result<Option<CodingCompletion>, CodingError> {
        let mut cursor = None;
        let mut latest = None;
        loop {
            let page = self
                .application
                .store
                .list_coding_completion_history(self.job_id(), cursor, 32)
                .await
                .map_err(CodingError::owner)?;
            for item in &page.items {
                if item.result.agent_id() == agent {
                    latest = Some(item.clone());
                }
            }
            let Some(next) = page.next_cursor else {
                break;
            };
            cursor = Some(next);
        }
        Ok(latest)
    }

    /// Finds the latest recorded completion for one agent, which every finished
    /// child has: a terminal child always recorded one.
    pub(crate) async fn find_completion(
        &self,
        agent: &AgentId,
    ) -> Result<CodingCompletion, CodingError> {
        self.latest_completion(agent)
            .await?
            .ok_or(CodingError::NotOwned)
    }
}
