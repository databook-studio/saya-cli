//! The notification settings the surface reads: whether anything is said at
//! all, and how long a turn must run before its end is news.
//!
//! One switch and one threshold, both configuration (`[ui] notifications`,
//! `[ui] notify_after_seconds`). The default is on, and a turn of thirty
//! seconds or more is long enough to notify about.

use std::time::Duration;

use super::model::Model;

/// The turn length a notification waits for when the configuration states
/// none.
pub(crate) const DEFAULT_AFTER: Duration = Duration::from_secs(30);

/// Whether the surface notifies, and how long a turn must run first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Notify {
    /// The switch. Off means no bell and no desktop notification at all.
    pub(crate) enabled: bool,
    /// The length after which a turn's end is worth a notification.
    pub(crate) after: Duration,
}

impl Default for Notify {
    fn default() -> Self {
        Self {
            enabled: true,
            after: DEFAULT_AFTER,
        }
    }
}

impl Model {
    /// The notification settings in force: the configuration's, or the default
    /// when the launch stated none.
    pub(crate) fn notifications(&self) -> Notify {
        self.notify.unwrap_or_default()
    }

    /// Whether the operator asked to be notified at all.
    pub(crate) fn notifications_enabled(&self) -> bool {
        self.notifications().enabled
    }

    /// How long a turn must run before its end is worth a notification.
    pub(crate) fn notify_after(&self) -> Duration {
        self.notifications().after
    }
}
