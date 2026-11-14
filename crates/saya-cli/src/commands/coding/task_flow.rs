//! The direct-mode `helpers` tool orchestration: one call launches one helper
//! into the job's live schedule session, waits for THAT helper's own completion,
//! and returns its redacted summary. Several calls of one turn run at the same
//! time, each returning its own helper's summary. Helpers work in the root's own
//! working checkout W, so there is no per-helper checkout.
//!
//! `task_run` is `delegate` (stage + admit) + the helper's own waiter; the
//! per-helper summary lives in `task_merge`. Direct mode only — the `helpers`
//! tool is advertised solely by `DirectCodingTools`.

use super::delegate::CodingTask;
use super::task_merge_record::ChildSummary;
use super::{CodingError, NativeCodingJob};
use saya_harness::coding::coordinator::ChildOutcome;
use saya_types::{AgentId, CodingResultOutcome};
use serde_json::Value;
use std::sync::atomic::Ordering;
use tokio::sync::oneshot;

/// The helper assignment cap: the description, the blank line, and the
/// prompt. Defined here, next to `one_helper`, because the `helpers` tool owns
/// the bound; `delegate` re-checks it.
pub(crate) const MAX_HELPER_ASSIGNMENT_BYTES: usize = 32 * 1024;

impl NativeCodingJob {
    /// Runs one `helpers` call: one helper, then its OWN completion and
    /// summary. Direct mode only.
    pub(crate) async fn task_run(
        self: &std::sync::Arc<Self>,
        args: Value,
    ) -> Result<Value, CodingError> {
        let task = one_helper(self, &args)?;
        let waiters = self.delegate(vec![task]).await?;
        self.summarize_helpers(waiters).await
    }

    /// The post-delegate half of `task_run`: await each helper's own completion
    /// waiter and return a redacted summary per helper, then release the batch
    /// from the job's live session. Helpers write W directly, so there is
    /// nothing to merge. A waiter that never fires means that helper never
    /// started (the session stopped while it was queued): it is summarized as
    /// failed rather than awaited forever.
    async fn summarize_helpers(
        &self,
        waiters: Vec<(AgentId, oneshot::Receiver<ChildOutcome>)>,
    ) -> Result<Value, CodingError> {
        // The release runs whatever the summaries did: a helper that finished
        // must leave the session's in-flight count even when its summary could
        // not be read.
        let launched = waiters.len();
        let summaries = self.collect_summaries(waiters).await;
        self.release_helpers(launched).await;
        summaries
    }

    async fn collect_summaries(
        &self,
        waiters: Vec<(AgentId, oneshot::Receiver<ChildOutcome>)>,
    ) -> Result<Value, CodingError> {
        let mut summaries = Vec::with_capacity(waiters.len());
        for (agent, waiter) in waiters {
            let summary = match waiter.await {
                Ok(_) => self.child_summary(&agent).await?,
                Err(_) => not_started_summary(&agent),
            };
            summaries.push(serde_json::to_value(&summary).map_err(CodingError::owner)?);
        }
        Ok(Value::Array(summaries))
    }
}

/// The summary of a helper that never started: its waiter was dropped before
/// it could run, so there is no durable completion to read.
fn not_started_summary(agent: &AgentId) -> ChildSummary {
    ChildSummary {
        agent: agent.clone(),
        outcome: CodingResultOutcome::Failed,
        failure_class: None,
        summary: "the helper never started: the schedule session stopped before it ran".into(),
        summary_truncated: false,
        assignment_added: Vec::new(),
        authority: "none",
    }
}

/// One `helpers` call is one helper. `subagent_type` names the helper type,
/// which admission then checks against the job's registry; every helper writes
/// the same checkout, so none declares paths. `agent_id` is optional; a
/// missing one is assigned from how many children this job has already
/// launched.
///
/// `write_paths` is tolerated and ignored, not read: helpers share W, so
/// declared paths mean nothing. It stays accepted because scripted fixtures
/// still send it.
fn one_helper(job: &NativeCodingJob, args: &Value) -> Result<CodingTask, CodingError> {
    let object = args.as_object().ok_or(CodingError::InvalidRequest)?;
    if object.keys().any(|key| {
        !matches!(
            key.as_str(),
            "description" | "prompt" | "subagent_type" | "agent_id" | "write_paths"
        )
    }) {
        return Err(CodingError::InvalidRequest);
    }
    let description = object
        .get("description")
        .and_then(Value::as_str)
        .filter(|text| {
            !text.is_empty() && text.len() <= super::delegate::MAX_HELPER_DESCRIPTION_BYTES
        })
        .ok_or(CodingError::InvalidRequest)?;
    let prompt = object
        .get("prompt")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or(CodingError::InvalidRequest)?;
    let assignment = format!("{description}\n\n{prompt}");
    if assignment.len() > MAX_HELPER_ASSIGNMENT_BYTES {
        return Err(CodingError::AssignmentTooLong);
    }
    // The type the helper is composed as. Whether this job advertises it is
    // decided at admission (`validate_delegate_tasks`), which refuses a name
    // the registry does not hold by name, with the ones it does.
    let subagent_type = object
        .get("subagent_type")
        .and_then(Value::as_str)
        .ok_or(CodingError::InvalidRequest)?
        .to_owned();
    let agent_id = match object.get("agent_id").and_then(Value::as_str) {
        Some(id) => AgentId::parse(id).map_err(|_| CodingError::InvalidRequest)?,
        None => {
            let n = job
                .delegated_children
                .load(Ordering::Acquire)
                .saturating_add(1);
            AgentId::parse(&format!("helper-{n}")).map_err(CodingError::owner)?
        }
    };
    Ok(CodingTask {
        agent_id,
        task: assignment,
        subagent_type,
        write_paths: Vec::new(),
        dependencies: Vec::new(),
    })
}
