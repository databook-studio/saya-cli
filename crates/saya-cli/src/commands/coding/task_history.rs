use super::{CodingError, CodingTask};
use saya_types::{AgentId, JobState};
use std::{collections::BTreeMap, path::Path};

pub(super) const MAX_TASK_HISTORY_BYTES: usize = 1024 * 1024;
pub(super) const MAX_TASKS: usize = 256;

/// The delegated tasks a job recorded, one batch per line, keyed by agent
/// with the byte size of the history. Oversized, malformed or duplicated
/// history is refused rather than partly trusted.
pub(super) fn read_tasks(
    records: &Path,
) -> Result<(BTreeMap<AgentId, CodingTask>, usize), CodingError> {
    let path = records.join("tasks.jsonl");
    let bytes = if path.exists() {
        super::context::read(&path, MAX_TASK_HISTORY_BYTES)?
    } else {
        Vec::new()
    };
    let mut tasks = BTreeMap::new();
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let batch: Vec<CodingTask> = serde_json::from_slice(line).map_err(CodingError::owner)?;
        for task in batch {
            if task.task.len() > 16 * 1024
                || task.dependencies.len() > 8
                || tasks.insert(task.agent_id.clone(), task).is_some()
                || tasks.len() > MAX_TASKS
            {
                return Err(CodingError::InvalidRequest);
            }
        }
    }
    Ok((tasks, bytes.len()))
}

/// The dependencies a still-unfinished child is waiting on: those that have
/// no terminal child record yet. A finished child waits on nothing.
pub(super) fn blocked_by<'a>(
    task: &'a CodingTask,
    own_state: JobState,
    states: &BTreeMap<AgentId, JobState>,
) -> Vec<&'a AgentId> {
    if super::continuation::is_terminal(own_state) {
        return Vec::new();
    }
    task.dependencies
        .iter()
        .filter(|dependency| {
            !states
                .get(*dependency)
                .copied()
                .is_some_and(super::continuation::is_terminal)
        })
        .collect()
}

#[cfg(test)]
#[path = "task_history_tests.rs"]
mod task_history_tests;
