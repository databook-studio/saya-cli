//! Assembly of the untrusted continuation context body for a native coding job.

use super::super::{
    CodingError, NativeCodingJob, conflict_record, conflicts, context, create::digest, task_history,
};
use super::is_terminal;
use saya_types::JobState;
use serde_json::{Value, json};
use std::{collections::BTreeMap, num::NonZeroU64};

const MAX_CONTINUATION_BYTES: usize = 48 * 1024;
const DECISION_CONTENT_BYTES: usize = 256;

impl NativeCodingJob {
    pub(super) async fn pending_tasks(&self) -> Result<(Vec<Value>, usize, usize), CodingError> {
        let (tasks, history_bytes) = task_history::read_tasks(&self.records)?;
        let mut cursor = None;
        let mut states = BTreeMap::new();
        loop {
            let page = self
                .application
                .store
                .list_coding_children(self.job_id(), cursor.as_ref(), 100)
                .await
                .map_err(CodingError::owner)?;
            states.extend(
                page.items
                    .into_iter()
                    .map(|child| (child.agent_id, child.state)),
            );
            let Some(next) = page.next_cursor else { break };
            cursor = Some(next);
            if states.len() > task_history::MAX_TASKS {
                return Err(CodingError::InvalidRequest);
            }
        }
        let mut completed = 0;
        let pending = tasks
            .into_iter()
            .filter_map(|(agent_id, task)| {
                let state = states.get(&agent_id).copied();
                if state.is_some_and(is_terminal) {
                    completed += 1;
                    return None;
                }
                Some(json!({
                    "agent_id": agent_id,
                    "state": state.map(state_value).unwrap_or_else(|| json!("not_started")),
                    "dependencies": task.dependencies
                }))
            })
            .collect();
        Ok((pending, history_bytes, completed))
    }

    pub(super) async fn accepted_decisions(&self) -> Result<(Vec<Value>, usize), CodingError> {
        let mut after: Option<NonZeroU64> = None;
        let mut accepted = Vec::new();
        let mut unaccepted = 0;
        loop {
            let page = self
                .application
                .store
                .list_coding_completion_history(self.job_id(), after, 32)
                .await
                .map_err(CodingError::owner)?;
            for completion in page.items {
                if completion.acknowledged {
                    let (content, truncated) =
                        prefix(completion.result.answer(), DECISION_CONTENT_BYTES);
                    accepted.push(json!({
                        "sequence": completion.sequence,
                        "agent_id": completion.result.agent_id(),
                        "outcome": completion.result.outcome(),
                        "content": content,
                        "content_truncated": truncated || completion.result.answer_truncated()
                    }));
                } else {
                    unaccepted += 1;
                }
            }
            let Some(next) = page.next_cursor else { break };
            after = Some(next);
        }
        Ok((accepted, unaccepted))
    }

    pub(super) fn pending_conflicts(&self) -> Result<Vec<Value>, CodingError> {
        let path = self.records.join("conflicts.jsonl");
        if !path.exists() {
            return Ok(Vec::new());
        }
        context::read(&path, conflicts::MAX_LOG_BYTES)?
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| {
                let record = conflict_record::parse_retained_record(line, self.job_id())?;
                let record_sha256_scope = if record.version == 2 {
                    "canonical_record_without_record_sha256"
                } else {
                    "raw_json_line"
                };
                let record_sha256 = record.record_sha256.unwrap_or(digest(line)?.to_string());
                Ok(json!({
                    "candidate_checkout_id": record.candidate_checkout_id,
                    "record_sha256": record_sha256,
                    "record_sha256_scope": record_sha256_scope,
                    "record_status_at_capture": record.status,
                    "current_state": "unknown_requires_fresh_revalidation",
                    "authority": "none",
                    "resolution": "not_implemented"
                }))
            })
            .collect()
    }
}

fn state_value(state: JobState) -> Value {
    serde_json::to_value(state).expect("JobState serializes")
}

fn prefix(value: &str, maximum: usize) -> (String, bool) {
    if value.len() <= maximum {
        return (value.into(), false);
    }
    let mut end = maximum;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    (value[..end].into(), true)
}

pub(super) fn bounded_body(mut projection: Value) -> Result<String, CodingError> {
    let mut omitted_content = 0;
    loop {
        projection["omitted"]["accepted_decision_content"] = json!(omitted_content);
        let body = serde_json::to_string(&projection).map_err(CodingError::owner)?;
        if body.len() <= MAX_CONTINUATION_BYTES {
            return Ok(body);
        }
        let Some(decision) =
            projection["accepted_decisions"]
                .as_array_mut()
                .and_then(|decisions| {
                    decisions
                        .iter_mut()
                        .rev()
                        .find(|decision| decision["content"].is_string())
                })
        else {
            return Err(CodingError::InvalidRequest);
        };
        decision
            .as_object_mut()
            .expect("JSON object")
            .remove("content");
        decision["content_truncated"] = Value::Bool(true);
        omitted_content += 1;
    }
}
