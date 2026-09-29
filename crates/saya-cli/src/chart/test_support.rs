//! Test-only serialization for the process-global session chart registry.
//!
//! The registry in `cleanup` is process-global, and lib tests run as parallel
//! threads of one process, so a test that drains the registry while another
//! test's chart is mid-flight deletes that chart — and a reservation made
//! between a drain test's two cleanups breaks its drain counts. Every lib
//! test that reserves or drains session charts holds
//! [`lock_session_charts_for_test`] for its whole body.

use std::sync::{Mutex, MutexGuard};

static SESSION_CHART_TEST_LOCK: Mutex<()> = Mutex::new(());

/// Guard for [`lock_session_charts_for_test`].
///
/// Deliberately a named wrapper around `MutexGuard`, not the bare guard: two
/// `render_chart` tests are `#[tokio::test]` bodies that must hold the lock
/// across their `await` points (the chart reservation happens inside the
/// awaited tool call, and a concurrent drain must not delete it mid-flight).
/// Each of those tests runs on its own single-threaded tokio runtime and no
/// other task on that runtime takes this lock, so holding it across `await`
/// cannot block anything; clippy's `await_holding_lock` flags the bare std
/// guard, whose hazard does not exist in this context.
pub(crate) struct SessionChartTestLock(MutexGuard<'static, ()>);

impl std::ops::Deref for SessionChartTestLock {
    type Target = ();

    fn deref(&self) -> &() {
        &self.0
    }
}

/// Serializes every test that reserves or drains the session chart registry.
pub(crate) fn lock_session_charts_for_test() -> SessionChartTestLock {
    SessionChartTestLock(
        SESSION_CHART_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    )
}
