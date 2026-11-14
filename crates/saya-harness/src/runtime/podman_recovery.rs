//! Bounded, cancellable recovery of interrupted container probes.

use std::time::{Duration, Instant};

use saya_agent::CancellationToken;

use super::super::{
    CleanupStatus,
    podman_cleanup::{CleanupContext, cleanup},
    registry::Registry,
};
use super::{Podman, PodmanError, ProbeFailure, probe::container_name};

const RECOVERY_BUDGET: Duration = Duration::from_secs(30);

impl Podman {
    pub(crate) async fn reconcile_pending(
        &self,
        registry: &Registry,
        cancellation: &CancellationToken,
    ) -> Result<(), PodmanError> {
        if cancellation.is_cancelled() {
            return Err(PodmanError::Cancelled);
        }
        let deadline = Instant::now() + RECOVERY_BUDGET;
        let tickets = registry
            .pending_probe_tickets()
            .map_err(|_| PodmanError::CleanupUnknown { operation: None })?;
        if cancellation.is_cancelled() {
            return Err(PodmanError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(PodmanError::CleanupUnknown {
                operation: Some(ProbeFailure::TimedOut),
            });
        }
        for ticket in tickets {
            if cancellation.is_cancelled() {
                return Err(PodmanError::CleanupUnknown {
                    operation: Some(ProbeFailure::Cancelled),
                });
            }
            if Instant::now() >= deadline {
                return Err(PodmanError::CleanupUnknown {
                    operation: Some(ProbeFailure::TimedOut),
                });
            }
            let name = container_name(&ticket);
            let cidfile = registry.root().join(format!("container-{ticket}"));
            let cleanup_context = CleanupContext::new(
                &self.executable,
                self.home.as_deref(),
                self.authfile().map_err(|_| PodmanError::Failed)?,
                deadline,
                cancellation,
            );
            let cleanup = cleanup(&cleanup_context, &cidfile, &name, &ticket, false).await;
            if cleanup == CleanupStatus::Unknown {
                let operation = if cancellation.is_cancelled() {
                    Some(ProbeFailure::Cancelled)
                } else if Instant::now() >= deadline {
                    Some(ProbeFailure::TimedOut)
                } else {
                    None
                };
                return Err(PodmanError::CleanupUnknown { operation });
            }
            self.finish_locator(registry, &cidfile, &ticket, None)?;
        }
        if cancellation.is_cancelled() {
            return Err(PodmanError::Cancelled);
        }
        Ok(())
    }
}
