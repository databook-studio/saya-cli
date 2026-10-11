use super::{CodingError, CodingReview, CodingTask, NativeCodingJob, context};
use saya_harness::coding::{
    integration::{
        AcceptedDelta, AssignedPaths, ConflictQueue, DEFAULT_DELTA_DEADLINE, EvidenceBinding,
        IntegrateError, IntegrateInput, compute_delta, evaluate_receipt, integrate_candidate,
    },
    worktree::verify_owned_checkout,
};
use saya_types::{JobState, ManagedCheckoutId};
impl NativeCodingJob {
    /// Builds one candidate from actual completed-child deltas under their exact
    /// recorded assignments. Overlaps remain typed conflicts with work retained.
    pub async fn review(&self, candidate: &ManagedCheckoutId) -> Result<CodingReview, CodingError> {
        if self.active.lock().await.is_some() {
            return Err(CodingError::Busy);
        }
        let host = &self.application;
        let children = host
            .store
            .list_coding_children(self.job_id(), None, 100)
            .await
            .map_err(CodingError::owner)?;
        if children.next_cursor.is_some()
            || children.items.iter().any(|child| {
                !matches!(
                    child.state,
                    JobState::Completed | JobState::Cancelled | JobState::Failed
                )
            })
        {
            return Err(CodingError::Busy);
        }
        let task_path = self.records.join("tasks.jsonl");
        let task_bytes = if task_path.exists() {
            context::read(&task_path, 1024 * 1024)?
        } else {
            Vec::new()
        };
        let mut tasks = Vec::<CodingTask>::new();
        for line in task_bytes
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
        {
            let batch: Vec<CodingTask> =
                serde_json::from_slice(line).map_err(CodingError::owner)?;
            tasks.extend(batch);
            if tasks.len() > 256 {
                return Err(CodingError::InvalidRequest);
            }
        }
        let records = host
            .store
            .list_managed_checkouts(self.job_id(), None, 100)
            .await
            .map_err(CodingError::owner)?;
        if records.next_cursor.is_some() {
            return Err(CodingError::InvalidRequest);
        }
        let baseline = host
            .artifacts
            .read_snapshot(self.plan.snapshot_id())
            .map_err(CodingError::owner)?;
        let mut sources = Vec::new();
        for record in records.items {
            let Some(agent) = record.ownership.agent_id() else {
                continue;
            };
            if !children
                .items
                .iter()
                .any(|child| &child.agent_id == agent && child.state == JobState::Completed)
            {
                continue;
            }
            let task = tasks
                .iter()
                .find(|task| &task.agent_id == agent)
                .ok_or(CodingError::NotOwned)?;
            let verified = verify_owned_checkout(
                &self.ownership(record.ownership.managed_checkout_id()),
                &self.cancellation,
            )
            .await
            .map_err(CodingError::owner)?;
            let delta = compute_delta(
                &verified,
                &baseline,
                &self.cancellation,
                DEFAULT_DELTA_DEADLINE,
            )
            .map_err(CodingError::owner)?;
            if delta.is_empty() {
                continue;
            }
            let assigned =
                AssignedPaths::parse(task.write_paths.clone()).map_err(CodingError::owner)?;
            let receipt = evaluate_receipt(&verified, &delta, &assigned, 64 * 1024 * 1024);
            let binding =
                EvidenceBinding::bind(&verified, &self.cancellation, DEFAULT_DELTA_DEADLINE)
                    .map_err(CodingError::owner)?;
            sources.push((verified, delta, receipt, binding));
        }
        self.create_checkout(candidate).await?;
        let verified = verify_owned_checkout(&self.ownership(candidate), &self.cancellation)
            .await
            .map_err(CodingError::owner)?;
        let candidate_binding =
            EvidenceBinding::bind(&verified, &self.cancellation, DEFAULT_DELTA_DEADLINE)
                .map_err(CodingError::owner)?;
        let accepted = sources
            .iter()
            .map(|(verified, delta, receipt, _)| AcceptedDelta::new(verified, receipt, delta))
            .collect::<Result<Vec<_>, _>>()
            .map_err(CodingError::owner)?;
        let result = integrate_candidate(
            &IntegrateInput {
                store: &host.store,
                sequencer: &self.ledger.sequencer(),
                job_id: self.job_id(),
                agent_id: None,
                candidate: &verified,
                baseline: &baseline,
                accepted: &accepted,
            },
            &self.cancellation,
            DEFAULT_DELTA_DEADLINE,
        )
        .await;
        match result {
            Ok(_) => {}
            Err(IntegrateError::Overlap(conflict)) => {
                let mut queue = ConflictQueue::new();
                queue
                    .record(candidate, &accepted, &conflict)
                    .map_err(CodingError::owner)?;
                let retained = queue
                    .open_for(candidate)
                    .ok_or(CodingError::InvalidRequest)?;
                let bindings = sources
                    .iter()
                    .map(|(_, _, _, binding)| binding.clone())
                    .collect::<Vec<_>>();
                let bytes = super::conflicts::retained_bytes(
                    self.job_id(),
                    retained,
                    &candidate_binding,
                    &bindings,
                )?;
                super::conflicts::validate_retained_bytes(&bytes, self.job_id())?;
                context::append_line(
                    &self.records.join("conflicts.jsonl"),
                    &bytes,
                    super::conflicts::MAX_LOG_BYTES,
                )?;
                return Err(CodingError::ConflictRetained {
                    job: self.job_id().to_string(),
                    candidate: candidate.to_string(),
                });
            }
            Err(error) => return Err(CodingError::owner(error)),
        }
        self.review_existing(candidate).await
    }
}
