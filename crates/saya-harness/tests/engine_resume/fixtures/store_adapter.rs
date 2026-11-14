/// A store adapter that refuses exactly the first `running` step mirror while
/// leaving the real journal and database available for assertions.
struct RefuseStepMirrorStore {
    inner: SqliteStateStore,
    refuse_status: RunStepStatus,
}

#[async_trait]
impl RunStore for RefuseStepMirrorStore {
    async fn create_run(&self, run: NewRun) -> Result<RunRecord, StoreError> {
        RunStore::create_run(&self.inner, run).await
    }

    async fn get_run(&self, id: &RunId) -> Result<Option<RunRecord>, StoreError> {
        RunStore::get_run(&self.inner, id).await
    }

    async fn list_runs(&self) -> Result<Vec<RunSummary>, StoreError> {
        RunStore::list_runs(&self.inner).await
    }

    async fn set_run_status(
        &self,
        id: &RunId,
        status: RunStatus,
        failure_code: Option<saya_types::RunFailureCode>,
    ) -> Result<(), StoreError> {
        RunStore::set_run_status(&self.inner, id, status, failure_code).await
    }

    async fn set_run_usage(&self, id: &RunId, usage: RunUsage) -> Result<(), StoreError> {
        RunStore::set_run_usage(&self.inner, id, usage).await
    }

    async fn upsert_step(
        &self,
        id: &RunId,
        step: usize,
        status: RunStepStatus,
    ) -> Result<(), StoreError> {
        if status == self.refuse_status {
            return Err(StoreError::unavailable());
        }
        RunStore::upsert_step(&self.inner, id, step, status).await
    }

    async fn set_step_usage(
        &self,
        id: &RunId,
        step: usize,
        usage: RunUsage,
    ) -> Result<(), StoreError> {
        RunStore::set_step_usage(&self.inner, id, step, usage).await
    }

    async fn list_steps(&self, id: &RunId) -> Result<Vec<RunStepRecord>, StoreError> {
        RunStore::list_steps(&self.inner, id).await
    }
}
