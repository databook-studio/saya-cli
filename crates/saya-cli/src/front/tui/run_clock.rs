//! The clock the runtime measures its frames on.
//!
//! Injected rather than read: the rate limit is the one place the loop looks at
//! the time, and a test that wants to prove "no frame is painted before 33 ms"
//! turns the clock by hand instead of waiting for it.

use std::time::Instant;

/// A reading the rate limit is measured on. It never runs backwards.
pub(crate) trait Clock: Send + Sync {
    /// The current reading.
    fn now(&self) -> Instant;
}

/// The machine's own clock.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}
