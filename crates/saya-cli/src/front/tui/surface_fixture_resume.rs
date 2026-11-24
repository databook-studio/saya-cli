//! The fixture's saved sessions: the records a case writes into its own store,
//! the rows the picker opens on, and the resume the picker's `↵` performs.

use std::path::PathBuf;
use std::sync::PoisonError;
use std::time::{Duration, Instant};

use super::run_cmd::CmdPort;
use super::surface_fixture::{Fixture, MODEL};
use crate::interactive::SessionState;

impl Fixture {
    /// Saves a record of `turns` in this fixture's own store, pinned to `folder`:
    /// the fixture's own when the case names none.
    pub(crate) async fn save_session(
        &self,
        id: &str,
        turns: &[(&str, &str)],
        folder: Option<PathBuf>,
    ) {
        use saya_store::SessionStore as _;
        let mut state = SessionState::new(id, None, MODEL);
        state.workspace_root = Some(
            folder
                .unwrap_or_else(|| self.folder())
                .to_string_lossy()
                .into_owned(),
        );
        for (user, assistant) in turns {
            state.record_turn(*user, *assistant, false, Vec::new());
        }
        self.deps()
            .store
            .save(state.redacted())
            .await
            .expect("the fixture's record saves");
    }

    /// The rows the picker opens on, as the surface itself lists them.
    pub(crate) async fn picker(&self) -> Vec<super::view::SessionRow> {
        super::sessions_picker::listed(&self.deps())
            .await
            .expect("the fixture's sessions list")
    }

    /// The rows the last `/sessions`-shaped command opened the picker with,
    /// waited for: the store's listing lands off the loop's thread.
    pub(crate) async fn picked(&self) -> Vec<super::view::SessionRow> {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(rows) = self
                .picked
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
            {
                return rows;
            }
            assert!(
                Instant::now() < deadline,
                "the surface never offered its saved sessions"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Resumes `id` the way the picker's `↵` does — the port's own operation —
    /// and waits for the line to land on the record.
    pub(crate) async fn resume(&self, id: &str) {
        self.port.resume_session(id.to_owned());
        let deadline = Instant::now() + Duration::from_secs(20);
        while self.line.busy() || self.pending() {
            assert!(
                Instant::now() < deadline,
                "the resume the surface was given never landed"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}
