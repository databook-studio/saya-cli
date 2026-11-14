//! Monitor lifetime follows the owned child future even if it panics or is dropped.
pub(crate) struct AbortMonitor(pub Option<tokio::task::JoinHandle<()>>);
impl AbortMonitor {
    pub(crate) async fn stop(mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
            let _ = task.await;
        }
    }
}
impl Drop for AbortMonitor {
    fn drop(&mut self) {
        if let Some(task) = &self.0 {
            task.abort();
        }
    }
}

pub(crate) struct InvocationGuard(pub Option<saya_agent::CancellationToken>);
impl Drop for InvocationGuard {
    fn drop(&mut self) {
        if let Some(token) = &self.0 {
            token.cancel_with_reason(saya_agent::CancellationReason::OwnerFailure);
        }
    }
}
