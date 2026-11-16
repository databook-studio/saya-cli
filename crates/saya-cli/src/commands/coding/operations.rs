use super::{CodingApplication, CodingError, NativeCodingJob};
use saya_harness::coding::snapshot::{MAX_SNAPSHOT_MANIFEST_BYTES, SnapshotManifest};
use saya_types::JobId;
use serde_json::{Value, json};

/// The non-fatal snapshot facts stored with the job's plan. A job whose
/// stored plan is absent or malformed reports none: this view is display
/// only, and the durable event stream remains the record. The summary count
/// states how many further sensitive paths the capped list does not name.
fn stored_snapshot_warnings(application: &CodingApplication, job: &JobId) -> (Vec<Value>, u64) {
    let path = application.state_root.join(job.as_str()).join("plan.json");
    let Ok(bytes) = super::context::read(&path, MAX_SNAPSHOT_MANIFEST_BYTES) else {
        return (Vec::new(), 0);
    };
    let Ok(digest) = super::create::digest(&bytes) else {
        return (Vec::new(), 0);
    };
    let Ok(manifest) = SnapshotManifest::from_verified_bytes(&bytes, &digest) else {
        return (Vec::new(), 0);
    };
    let warnings = manifest
        .warnings()
        .iter()
        .map(|warning| {
            json!({
                "path": warning.path().as_str(),
                "reason": warning.reason(),
            })
        })
        .collect();
    (warnings, manifest.warnings_truncated())
}
impl CodingApplication {
    pub async fn status(&self, job: &JobId) -> Result<Value, CodingError> {
        let record = self.owned_job(job).await?;
        let children = self
            .store
            .list_coding_children(job, None, 100)
            .await
            .map_err(CodingError::owner)?;
        let checkouts = self
            .store
            .list_managed_checkouts(job, None, 100)
            .await
            .map_err(CodingError::owner)?;
        let tasks = super::task_history::read_tasks(&self.state_root.join(job.as_str()))
            .map(|(tasks, _)| tasks)?;
        let states: std::collections::BTreeMap<_, _> = children
            .items
            .iter()
            .map(|child| (child.agent_id.clone(), child.state))
            .collect();
        let mut lineage = std::collections::BTreeMap::new();
        for child in &children.items {
            let history = self
                .store
                .coding_child_invocation_history(job, &child.agent_id)
                .await
                .map_err(CodingError::owner)?;
            let bound=history.iter().any(|event|matches!(event.event(),saya_types::CodingEventKind::ChildInvocation{record} if matches!(record.as_ref(),saya_types::ChildInvocationRecord::ScopeBound{..})));
            let follow_ups=history.iter().filter(|event|matches!(event.event(),saya_types::CodingEventKind::ChildInvocation{record} if matches!(record.as_ref(),saya_types::ChildInvocationRecord::FollowUpPrepared{..}))).count();
            lineage.insert(child.agent_id.clone(),json!({"scope_bound":bound,"follow_ups_prepared":follow_ups,"maximum_follow_ups":saya_types::MAX_CHILD_FOLLOW_UPS,"records":history,"authority":"none"}));
        }
        let completions = self
            .store
            .list_coding_completion_history(job, None, 32)
            .await
            .map_err(CodingError::owner)?;
        let child_views = children
            .items
            .iter()
            .map(|child| {
                let task = tasks.get(&child.agent_id);
                // With more than one page of children a dependency's record can
                // sit on a page not read here: unknown, never a guessed wait.
                let blocked_by = task
                    .filter(|_| children.next_cursor.is_none())
                    .map(|task| super::task_history::blocked_by(task, child.state, &states));
                json!({"agent_id":child.agent_id,"state":child.state,"invocation_id":child.current_invocation_id,
                    "write_paths":task.map(|task| &task.write_paths),
                    "dependencies":task.map(|task| &task.dependencies),
                    "invocation_lineage":lineage.get(&child.agent_id),
                    "blocked_by":blocked_by})
            })
            .collect::<Vec<_>>();
        let failure = self
            .store
            .latest_coding_run_event(job)
            .await
            .map_err(CodingError::owner)?
            .filter(|event| {
                matches!(
                    event.event(),
                    saya_types::CodingEventKind::RunTerminated { .. }
                )
            });
        let diagnostic_history =
            match super::diagnostics::history(&self.state_root.join(job.as_str())) {
                Ok(history) => serde_json::to_value(history).map_err(CodingError::owner)?,
                Err(fault) => json!({"state":"unknown","fault":fault}),
            };
        let usage = super::usage::read(self, job).await?;
        let progress = super::progress_report::read(&self.state_root.join(job.as_str()));
        let outcome = super::outcome::read(&self.state_root.join(job.as_str()));
        // An in-place job reports its fixed baseline, its own omissions, and
        // its changes under their own keys; the included-sensitive warning
        // list above stays specific to Git jobs.
        let in_place = super::in_place_status::view(self, job, &record, outcome.as_ref())?;
        let (snapshot_warnings, snapshot_warnings_truncated) = stored_snapshot_warnings(self, job);
        let mut status = json!({"job_id":record.job_id,"state":record.state,"root_mode":self.root_mode_name(),"outcome":outcome,"failure":failure,"usage":usage,"progress":progress,"snapshot_warnings":snapshot_warnings,"snapshot_warnings_truncated":snapshot_warnings_truncated,"diagnostic_history":diagnostic_history,"children":child_views,"next_cursor":children.next_cursor,
                "checkouts":checkouts.items.iter().map(|record|json!({"checkout_id":record.ownership.managed_checkout_id(),"agent_id":record.ownership.agent_id(),"lifecycle":record.current_lifecycle})).collect::<Vec<_>>(),
                "completions":completions.items.iter().map(|item|json!({"sequence":item.sequence,"agent_id":item.result.agent_id(),"invocation_id":item.result.invocation_id(),"outcome":item.result.outcome(),"acknowledged":item.acknowledged})).collect::<Vec<_>>(),"completions_next_cursor":completions.next_cursor,
                "checkouts_next_cursor":checkouts.next_cursor});
        if let Some(Value::Object(facts)) = in_place {
            let object = status.as_object_mut().ok_or(CodingError::InvalidRequest)?;
            for (key, value) in facts {
                object.insert(key, value);
            }
        }
        if let Some(reason) = super::root_mode::read_fallback(&self.state_root.join(job.as_str())) {
            status["managed_fallback"] = json!({"reason": reason});
        }
        Ok(status)
    }
    pub async fn cancel(&self, job: &JobId) -> Result<Value, CodingError> {
        self.owned_job(job).await?;
        let recorded = self
            .store
            .request_coding_cancel(job)
            .await
            .map_err(CodingError::owner)?;
        Ok(json!({"job_id":job,"cancel_requested":true,"new_request":recorded}))
    }
    // Cancellation requires direct job/repository ownership, independent of
    // derived status/history decoding. Status still validates those records.
    /// The repository id this application owns jobs under: the Git
    /// repository's durable identity, or the in-place root's local id. It is
    /// the one filter `owned_job` and `list` share, so an in-place job is
    /// owned by the folder it was started in and by no other.
    async fn root_identity(&self) -> Result<saya_types::RepositoryId, CodingError> {
        if let Some(local) = self.local_root() {
            return Ok(local.id().as_repository_id().clone());
        }
        let repository = self.git_repository("list jobs")?;
        let identity = self
            .service
            .inventory_repository(repository, &saya_agent::CancellationToken::new())
            .await
            .map_err(CodingError::owner)?;
        Ok(identity.identity().repository_id().clone())
    }
    pub(crate) async fn owned_job(
        &self,
        job: &JobId,
    ) -> Result<saya_store::CodingJobRecord, CodingError> {
        let record = self
            .store
            .get_coding_job(job)
            .await
            .map_err(CodingError::owner)?
            .ok_or(CodingError::NotOwned)?;
        if record.repository_id != self.root_identity().await? {
            return Err(CodingError::NotOwned);
        }
        Ok(record)
    }
    pub async fn list(&self, after: Option<&JobId>) -> Result<Value, CodingError> {
        let page = self
            .store
            .list_coding_jobs(after, 100)
            .await
            .map_err(CodingError::owner)?;
        let identity = self.root_identity().await?;
        let git_identity = self.git_identity_best_effort().await;
        Ok(
            json!({"jobs":page.items.iter().filter(|record| record.repository_id==identity||git_identity.as_ref()==Some(&record.repository_id)).map(|record|json!({"job_id":record.job_id,"state":record.state})).collect::<Vec<_>>(),"next_cursor":page.next_cursor}),
        )
    }

    /// The Git repository identity when an in-place root sits inside one, so a
    /// listing sees the jobs started under the repository as well as those
    /// started under the folder. Best effort: a plain folder or any Git
    /// refusal yields `None` and the listing stays folder-only.
    async fn git_identity_best_effort(&self) -> Option<saya_types::RepositoryId> {
        let local = self.local_root()?;
        let repository = self
            .service
            .open_repository(local.path(), &saya_agent::CancellationToken::new())
            .await
            .ok()?;
        let identity = self
            .service
            .inventory_repository(&repository, &saya_agent::CancellationToken::new())
            .await
            .ok()?;
        Some(identity.identity().repository_id().clone())
    }
}
impl NativeCodingJob {
    pub async fn status(&self) -> Result<Value, CodingError> {
        let mut status = self.application.status(self.job_id()).await?;
        let readiness = self.candidate_readiness().await.into_view();
        status["readiness"] = readiness.clone();
        status["next_action"] = readiness["next_action"].clone();
        Ok(status)
    }
}
