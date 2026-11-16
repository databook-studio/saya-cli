//! The dispatched filesystem worker runs the prepared removal to completion.
use saya_agent::{CancellationReason, CancellationToken};
use saya_harness::workspace::{ControlledRemoveError, Workspace};
use saya_types::{Sha256Digest, WorkspacePath};
use std::time::{Duration, Instant};

struct CancelOnDrop(Option<CancellationToken>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(token) = self.0.take() {
            token.cancel_with_reason(CancellationReason::OwnerFailure);
        }
    }
}

pub(super) async fn run(
    workspace: Workspace,
    path: WorkspacePath,
    expected: Sha256Digest,
    cancellation: CancellationToken,
) -> Result<(), ControlledRemoveError> {
    // Use the caller's actual token at the publication boundary, rather than
    // a separately forwarded token that can lag a cancellation before dispatch.
    let mut stop = CancelOnDrop(Some(cancellation.clone()));
    let worker = tokio::task::spawn_blocking(move || {
        workspace
            .prepare_guarded_remove(
                path.as_str(),
                expected.as_str(),
                cancellation,
                Instant::now() + Duration::from_secs(10),
            )?
            .commit()
    });
    let result = worker.await;
    stop.0.take();
    result.unwrap_or(Err(ControlledRemoveError::Unknown))
}

#[cfg(all(test, unix))]
#[path = "worker_tests.rs"]
mod tests;
