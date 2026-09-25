//! Applying the modal's answer onto `App`.

use super::super::transcript::BlockKind;
use super::super::types::App;
use super::keys::{TrustResolution, trust_key};
use crate::interactive::session_trust;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

impl App {
    /// Applies one trust-modal key press: resolves the key and — where the
    /// answer names a directory — closes the modal with the trust echo and
    /// stashes the directory for the event loop, which recomposes the live
    /// runtime behind the app's universe snapshot. Closing the modal
    /// unbound binds nothing — today's shape. Returns the trusted directory
    /// when the answer bound one; the event loop drains it exactly once
    /// through `take_trust_answer`.
    pub(crate) fn answer_trust(
        &mut self,
        code: KeyCode,
        _mods: KeyModifiers,
    ) -> Option<std::path::PathBuf> {
        let prompt = self.overlays.trust.as_mut()?;
        let resolution = trust_key(prompt, code)?;
        match resolution {
            TrustResolution::ContinueUnbound => {
                self.overlays.trust = None;
                None
            }
            TrustResolution::TrustCwd => {
                let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                let dir = session_trust::resolve_trusted_dir(&cwd).unwrap_or(cwd);
                self.close_trust_with_dir(&dir);
                Some(dir)
            }
            TrustResolution::Workspace(dir) => {
                self.close_trust_with_dir(&dir);
                Some(dir)
            }
        }
    }

    /// Closes the trust modal saying the trust echo into the transcript —
    /// the half of the moment-of-choice pair the lane fact does not carry.
    fn close_trust_with_dir(&mut self, dir: &std::path::Path) {
        self.overlays.trust = None;
        self.pending_trust_answer = Some(dir.to_path_buf());
        self.transcript
            .push(BlockKind::System, session_trust::trusted_root_line(dir));
    }
}

impl App {
    /// Drains the stashed trust answer exactly once: the event loop calls
    /// this after `answer_trust` closed the modal with a bound directory.
    pub(crate) fn take_trust_answer(&mut self) -> Option<std::path::PathBuf> {
        self.pending_trust_answer.take()
    }
}
