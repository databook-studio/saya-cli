//! The session a job belongs to, when the surface opening it states one.
//!
//! A job a chat session opens over the session's own folder runs *in* that
//! folder, alongside the session's run — the way a subagent works in its
//! parent's working directory. The folder's one lease therefore names the
//! session, and a job that states that same session is admitted as its
//! dependent instead of refused. Isolation stays opt-in: a job started in a
//! checkout of its own states no session here, and takes no lease over the
//! session's folder at all.
//!
//! The cell is set by the surface before the job exists and read only when the
//! job opens, so it is never authority over a running job.

use std::sync::PoisonError;

use super::CodingApplication;

impl CodingApplication {
    /// States the session this application's job is a job of — `None` for a
    /// job that belongs to no session (the headless launch, a lifecycle verb).
    pub fn set_session_parent(&self, session: Option<&str>) {
        *self
            .session_parent
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = session.map(str::to_owned);
    }

    /// The session this application's job is a job of, if a surface stated one.
    pub(super) fn session_parent(&self) -> Option<String> {
        self.session_parent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}
