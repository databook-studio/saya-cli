use super::{CodingApplication, CodingError, NativeCodingJob};
use saya_types::JobId;
use serde_json::{Value, json};
impl CodingApplication {
    pub async fn status(&self, job: &JobId) -> Result<Value, CodingError> {
        let record = self
            .store
            .get_coding_job(job)
            .await
            .map_err(CodingError::owner)?
            .ok_or(CodingError::NotOwned)?;
        let identity = self
            .service
            .inventory_repository(&self.repository, &saya_agent::CancellationToken::new())
            .await
            .map_err(CodingError::owner)?;
        if record.repository_id != *identity.identity().repository_id() {
            return Err(CodingError::NotOwned);
        }
        let children = self
            .store
            .list_coding_children(job, None, 100)
            .await
            .map_err(CodingError::owner)?;
        // The tasks are read for context only: unreadable history leaves the
        // task fields null instead of hiding the job's state.
        let tasks = super::task_history::read_tasks(&self.state_root.join(job.as_str()))
            .map(|(tasks, _)| tasks)
            .ok();
        let states: std::collections::BTreeMap<_, _> = children
            .items
            .iter()
            .map(|child| (child.agent_id.clone(), child.state))
            .collect();
        let child_views = children
            .items
            .iter()
            .map(|child| {
                let task = tasks.as_ref().and_then(|tasks| tasks.get(&child.agent_id));
                // With more than one page of children a dependency's record can
                // sit on a page not read here: unknown, never a guessed wait.
                let blocked_by = task
                    .filter(|_| children.next_cursor.is_none())
                    .map(|task| super::task_history::blocked_by(task, child.state, &states));
                json!({"agent_id":child.agent_id,"state":child.state,"invocation_id":child.current_invocation_id,
                    "write_paths":task.map(|task| &task.write_paths),
                    "dependencies":task.map(|task| &task.dependencies),
                    "blocked_by":blocked_by})
            })
            .collect::<Vec<_>>();
        Ok(
            json!({"job_id":record.job_id,"state":record.state,"children":child_views,"next_cursor":children.next_cursor}),
        )
    }
    pub async fn cancel(&self, job: &JobId) -> Result<Value, CodingError> {
        self.status(job).await?;
        let recorded = self
            .store
            .request_coding_cancel(job)
            .await
            .map_err(CodingError::owner)?;
        Ok(json!({"job_id":job,"cancel_requested":true,"new_request":recorded}))
    }
    pub async fn list(&self, after: Option<&JobId>) -> Result<Value, CodingError> {
        let page = self
            .store
            .list_coding_jobs(after, 100)
            .await
            .map_err(CodingError::owner)?;
        let identity = self
            .service
            .inventory_repository(&self.repository, &saya_agent::CancellationToken::new())
            .await
            .map_err(CodingError::owner)?;
        Ok(
            json!({"jobs":page.items.iter().filter(|record| &record.repository_id==identity.identity().repository_id()).map(|record|json!({"job_id":record.job_id,"state":record.state})).collect::<Vec<_>>(),"next_cursor":page.next_cursor}),
        )
    }
}
impl NativeCodingJob {
    pub async fn status(&self) -> Result<Value, CodingError> {
        self.application.status(self.job_id()).await
    }
}
