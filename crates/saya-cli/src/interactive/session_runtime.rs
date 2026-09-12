//! The live session's engine-side runtime: the composed tool universe and
//! the single-writer lock on its state directory, held for the process and
//! released at exit. Per SESSION-WORKSPACE.md, the lock rides the session
//! state (`sessions/<id>/lock`), never the user's project — two sessions on
//! one project are two writers among the editors and build tools that
//! already share the tree.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use saya_harness::lock::RunLock;

use super::session_paths::{create_state_dir, default_session_dir};
use super::session_universe::SessionUniverse;

pub(crate) struct SessionRuntime {
    universe: Arc<SessionUniverse>,
    /// Held, never read: the lock's purpose is its lifetime — dropping the
    /// runtime releases it.
    _lock: RunLock,
    /// The explicit `--workspace` statement, carried so a mid-session
    /// `/resume` re-binds the same way the launch did.
    explicit: Option<PathBuf>,
}

impl SessionRuntime {
    /// Acquires the session: the state directory (0700), the lock (refusing
    /// a live holder by pid, reclaiming a stale file), and the composed
    /// universe. `fresh` distinguishes a first start — the git worktree top
    /// binds when nothing is stated — from a resume, which re-opens the
    /// recorded pin and runs unbound when the record has none.
    pub(crate) fn acquire(
        runtime: &crate::config::runtime::RuntimeConfig,
        explicit: Option<&Path>,
        fresh: bool,
        pinned_root: Option<&str>,
        id: &str,
    ) -> Result<Self, String> {
        let state_dir = create_state_dir(&default_session_dir(), id)?;
        let lock = RunLock::acquire(state_dir.join("lock")).map_err(|error| match error {
            // The same words a run's second writer reads, with the same pid.
            saya_harness::HarnessError::LockHeld { pid } => format!(
                "session {id} is already running (pid {pid}): a session's scratch and \
                 transcript are single-writer; stop the other process or start a new session"
            ),
            other => format!("could not claim the session lock: {other}"),
        })?;
        let walk_when_unpinned = fresh;
        let universe = SessionUniverse::compose(
            runtime,
            explicit.map(Path::new),
            pinned_root,
            walk_when_unpinned,
            &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            &state_dir,
        )?;
        let explicit = explicit.map(Path::to_path_buf);
        Ok(Self {
            universe: Arc::new(universe),
            _lock: lock,
            explicit,
        })
    }

    /// The composed universe, shared with every turn of this session.
    pub(crate) fn universe(&self) -> Arc<SessionUniverse> {
        Arc::clone(&self.universe)
    }

    /// The canonical workspace root, when one binds.
    pub(crate) fn root(&self) -> Option<&Path> {
        self.universe.root()
    }

    /// The pinned root the session record should carry: the resolved root
    /// when this process bound one explicitly or freshly; `None` (keep the
    /// record's existing pin) when a resume re-opened a recorded root that
    /// has since vanished.
    pub(crate) fn record_root(&self, fresh: bool) -> Option<String> {
        if fresh || self.explicit.is_some() {
            return self.root().map(|root| root.display().to_string());
        }
        None
    }

    /// The startup notice the universe reported, if any — a vanished pin
    /// must be said, never silent.
    pub(crate) fn notice(&self) -> Option<&str> {
        self.universe.notice.as_deref()
    }

    /// Swaps the runtime to a resumed session id: the new state directory is
    /// claimed (refusing a live holder) and composed before the old lock
    /// releases, so a failed swap leaves the current session still held.
    pub(crate) fn reacquire(
        &mut self,
        runtime: &crate::config::runtime::RuntimeConfig,
        pinned_root: Option<&str>,
        id: &str,
    ) -> Result<(), String> {
        let replacement = Self::acquire(runtime, self.explicit.as_deref(), false, pinned_root, id)?;
        // Only reached when the new session is fully held and composed; the
        // swap drops this lock last.
        *self = replacement;
        Ok(())
    }
}
