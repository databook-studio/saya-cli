use std::time::Duration;

use tokio::time::Instant;

use super::command::GitError;
use super::repository::GitInventoryError;

pub(super) const INVENTORY_OPERATION_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy)]
pub(super) struct InventoryDeadline(Instant);

impl InventoryDeadline {
    pub(super) fn start() -> Self {
        Self(Instant::now() + INVENTORY_OPERATION_TIMEOUT)
    }

    pub(super) fn until(deadline: Instant) -> Self {
        Self(deadline)
    }

    pub(super) fn at(self) -> Instant {
        self.0
    }

    pub(super) fn check(self) -> Result<(), GitInventoryError> {
        self.remaining().map(|_| ())
    }

    pub(super) fn remaining(self) -> Result<Duration, GitInventoryError> {
        let remaining = self.0.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            Err(GitInventoryError::Git(GitError::TimedOut))
        } else {
            Ok(remaining)
        }
    }
}
