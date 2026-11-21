//! The workspace trust question in the new surface (`02` §9.4), and the store
//! an answer binds through.
//!
//! One decision, asked once: this session bound no workspace, so file tools are
//! unavailable and the host lane cannot compose. `y` binds the launch folder as
//! the session's workspace, `n` continues without one — the same two answers
//! today's modal gives through `t` and `c`, with the same effect, because the
//! store behind [`answer`] *is* today's call (`SessionRuntime::bind_trusted`,
//! exactly like an explicit `--workspace`).
//!
//! The question's words are today's own: [`BODY`] and [`NOTE`] are the one
//! trust module's constants, and the folder a `y` binds is resolved by today's
//! `resolve_trusted_dir` — never a parent, never a walk. What is new here is
//! only the key pair and the boundary the surface can be driven through
//! without acquiring a session.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use ratatui::crossterm::event::KeyCode;

use super::direct::DirectOutcome;
use super::msg::Worker;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session::permissions::trust as session_trust;
use crate::interactive::session::runtime::SessionRuntime;
use crate::interactive::tui::TrustResolution;

/// The fact and the question, in the one trust module's words (`02` §9.4).
pub(crate) const BODY: &str = session_trust::TRUST_PROMPT_BODY;

/// The session-only note under the answers, in the same words.
pub(crate) const NOTE: &str = session_trust::TRUST_PROMPT_NOTE;

/// The store a workspace answer binds through.
///
/// One method, because one call does the work: today's live session runtime
/// ([`SessionTrust`]). A test answers through a store that records instead, so
/// the question is asserted without acquiring a session.
pub(crate) trait TrustStore: Send + Sync {
    /// Bind `dir` as the session's workspace, exactly like `--workspace`.
    fn bind(&self, dir: &Path) -> Result<(), String>;
}

/// One press on the open question: `y` binds the launch folder, `n` (or Esc)
/// continues without one. Every other key leaves the question open.
pub(crate) fn press(key: KeyCode) -> Option<TrustResolution> {
    match key {
        KeyCode::Char('y') | KeyCode::Char('Y') => Some(TrustResolution::TrustCwd),
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            Some(TrustResolution::ContinueUnbound)
        }
        _ => None,
    }
}

/// Carries one answer out and says what the surface says for it: the trust
/// echo (`02` §3.10) once the folder bound, or why it could not. An answer
/// that binds nothing says nothing — today's shape.
pub(crate) fn answer(store: &dyn TrustStore, answer: TrustResolution) -> Worker {
    let dir = match answer {
        TrustResolution::ContinueUnbound => return Worker::default(),
        TrustResolution::TrustCwd => {
            let folder = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            match session_trust::resolve_trusted_dir(&folder) {
                Ok(resolved) => resolved,
                Err(message) => {
                    return Worker {
                        direct: Some(DirectOutcome::Error(message)),
                        ..Worker::default()
                    };
                }
            }
        }
        // The `w <dir>` answer is today's modal's, and binds the directory it
        // names — resolved by the same rule.
        TrustResolution::Workspace(dir) => match session_trust::resolve_trusted_dir(&dir) {
            Ok(resolved) => resolved,
            Err(message) => {
                return Worker {
                    direct: Some(DirectOutcome::Error(message)),
                    ..Worker::default()
                };
            }
        },
    };
    match store.bind(&dir) {
        Ok(()) => Worker {
            direct: Some(DirectOutcome::Notice(session_trust::trusted_root_line(
                &dir,
            ))),
            ..Worker::default()
        },
        Err(message) => Worker {
            direct: Some(DirectOutcome::Error(message)),
            ..Worker::default()
        },
    }
}

/// Today's store: the live session runtime's own `bind_trusted`, the call the
/// old surface makes when its modal closes on a directory.
pub(crate) struct SessionTrust {
    session: Mutex<SessionRuntime>,
    runtime: Arc<RuntimeConfig>,
    launch: crate::interactive::session_host::HostLaunch,
}

impl SessionTrust {
    /// A store over one live session, the composed runtime it was acquired
    /// with, and the launch statement the recomposition carries.
    pub(crate) fn new(
        session: SessionRuntime,
        runtime: Arc<RuntimeConfig>,
        launch: crate::interactive::session_host::HostLaunch,
    ) -> Self {
        Self {
            session: Mutex::new(session),
            runtime,
            launch,
        }
    }
}

impl TrustStore for SessionTrust {
    fn bind(&self, dir: &Path) -> Result<(), String> {
        let mut session = self.session.lock().unwrap_or_else(PoisonError::into_inner);
        session.bind_trusted(&self.runtime, dir, Some(&self.launch))
    }
}
