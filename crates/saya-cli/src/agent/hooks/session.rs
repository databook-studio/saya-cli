//! The `SessionStart` context, kept once per session process.
//!
//! The launch composes the engine and runs the hooks before the first turn;
//! every later turn's gate reads the bounded context from here rather than
//! running the hook again. A job root has no such cell: it is one process and
//! one loop, so it calls [`Hooks::session_start`] directly.
//!
//! [`Hooks::session_start`]: super::runner::Hooks::session_start

use std::sync::Arc;

use tokio::sync::OnceCell;

use super::runner::Hooks;

/// The context a `SessionStart` hook printed, computed at most once.
pub(crate) struct SessionHooks {
    context: OnceCell<Option<String>>,
}

impl SessionHooks {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            context: OnceCell::new(),
        })
    }

    /// Runs the `SessionStart` hooks once and keeps their bounded stdout. The
    /// first caller wins; a later or concurrent caller gets the same answer.
    pub(crate) async fn start(&self, hooks: &Hooks) -> Option<String> {
        self.context
            .get_or_init(|| async { hooks.session_start().await })
            .await
            .clone()
    }
}
