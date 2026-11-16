struct HoldFinalFailedMirror {
    inner: Arc<SqliteStateStore>,
    failures: AtomicUsize,
    entered: std::sync::Mutex<Option<oneshot::Sender<()>>>,
    release: std::sync::Mutex<Option<oneshot::Receiver<()>>>,
}

#[async_trait]
impl RunStore for HoldFinalFailedMirror {
    async fn create_run(&self, run: NewRun) -> Result<RunRecord, StoreError> {
        RunStore::create_run(self.inner.as_ref(), run).await
    }

    async fn get_run(&self, id: &RunId) -> Result<Option<RunRecord>, StoreError> {
        RunStore::get_run(self.inner.as_ref(), id).await
    }

    async fn list_runs(&self) -> Result<Vec<RunSummary>, StoreError> {
        RunStore::list_runs(self.inner.as_ref()).await
    }

    async fn set_run_status(
        &self,
        id: &RunId,
        status: RunStatus,
        failure_code: Option<saya_types::RunFailureCode>,
    ) -> Result<(), StoreError> {
        RunStore::set_run_status(self.inner.as_ref(), id, status, failure_code).await
    }

    async fn set_run_usage(&self, id: &RunId, usage: RunUsage) -> Result<(), StoreError> {
        RunStore::set_run_usage(self.inner.as_ref(), id, usage).await
    }

    async fn upsert_step(
        &self,
        id: &RunId,
        step: usize,
        status: RunStepStatus,
    ) -> Result<(), StoreError> {
        if status == RunStepStatus::Failed
            && self.failures.fetch_add(1, Ordering::SeqCst) + 1
                == saya_harness::engine::MAX_EPISODE_ATTEMPTS
        {
            let _ = self.entered.lock().unwrap().take().unwrap().send(());
            let release = self.release.lock().unwrap().take().unwrap();
            release.await.unwrap();
        }
        RunStore::upsert_step(self.inner.as_ref(), id, step, status).await
    }

    async fn set_step_usage(
        &self,
        id: &RunId,
        step: usize,
        usage: RunUsage,
    ) -> Result<(), StoreError> {
        RunStore::set_step_usage(self.inner.as_ref(), id, step, usage).await
    }

    async fn list_steps(&self, id: &RunId) -> Result<Vec<RunStepRecord>, StoreError> {
        RunStore::list_steps(self.inner.as_ref(), id).await
    }
}

struct FailStatusMirror {
    inner: Arc<SqliteStateStore>,
    fail_status: RunStatus,
}

#[async_trait]
impl RunStore for FailStatusMirror {
    async fn create_run(&self, run: NewRun) -> Result<RunRecord, StoreError> {
        RunStore::create_run(self.inner.as_ref(), run).await
    }

    async fn get_run(&self, id: &RunId) -> Result<Option<RunRecord>, StoreError> {
        RunStore::get_run(self.inner.as_ref(), id).await
    }

    async fn list_runs(&self) -> Result<Vec<RunSummary>, StoreError> {
        RunStore::list_runs(self.inner.as_ref()).await
    }

    async fn set_run_status(
        &self,
        id: &RunId,
        status: RunStatus,
        failure_code: Option<saya_types::RunFailureCode>,
    ) -> Result<(), StoreError> {
        if status == self.fail_status {
            return Err(StoreError::Unavailable);
        }
        RunStore::set_run_status(self.inner.as_ref(), id, status, failure_code).await
    }

    async fn set_run_usage(&self, id: &RunId, usage: RunUsage) -> Result<(), StoreError> {
        RunStore::set_run_usage(self.inner.as_ref(), id, usage).await
    }

    async fn upsert_step(
        &self,
        id: &RunId,
        step: usize,
        status: RunStepStatus,
    ) -> Result<(), StoreError> {
        RunStore::upsert_step(self.inner.as_ref(), id, step, status).await
    }

    async fn set_step_usage(
        &self,
        id: &RunId,
        step: usize,
        usage: RunUsage,
    ) -> Result<(), StoreError> {
        RunStore::set_step_usage(self.inner.as_ref(), id, step, usage).await
    }

    async fn list_steps(&self, id: &RunId) -> Result<Vec<RunStepRecord>, StoreError> {
        RunStore::list_steps(self.inner.as_ref(), id).await
    }
}
