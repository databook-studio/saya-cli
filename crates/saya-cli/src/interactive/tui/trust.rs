//! The startup trust modal's answers: the TUI's key handling for the one
//! trust decision — trust this folder, name another directory, or continue
//! unbound — opened once after the splash paints. A trust answer binds
//! through `SessionRuntime::bind_trusted`, exactly like an explicit
//! `--workspace`; a refusal or a continued-unbound answer closes the modal
//! with no bind. A bad directory refuses inline (the modal stays, with the
//! error) — never a launch failure, never a silent unbound session.

use super::transcript::BlockKind;
use super::types::App;
use crate::interactive::session_trust;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

/// The modal's answer once a key resolves to one: bind this folder, bind
/// the typed directory, or close unbound.
pub(crate) enum TrustResolution {
    /// Bind exactly the launch cwd, canonicalised — no parent, no walk.
    TrustCwd,
    /// Bind exactly the typed directory, already resolved.
    Workspace(std::path::PathBuf),
    /// Bind nothing: today's unbound shape.
    ContinueUnbound,
}

/// Answers one trust-modal key press. `t` trusts the launch cwd, `c` (or
/// Esc) continues unbound, and `w` starts (then commits, on Enter) the
/// `w <dir>` draft typed inline. Any other key feeds the open draft, and
/// Backspace edits it. Returns the resolution when the key closes the
/// modal; `None` while the modal stays open.
pub(crate) fn trust_key(
    prompt: &mut super::types::TrustPrompt,
    code: KeyCode,
) -> Option<TrustResolution> {
    let typing = prompt.draft.is_some();
    match code {
        KeyCode::Esc => {
            if typing {
                prompt.draft = None;
                prompt.error = None;
                None
            } else {
                Some(TrustResolution::ContinueUnbound)
            }
        }
        KeyCode::Backspace => {
            if let Some(draft) = prompt.draft.as_mut() {
                draft.pop();
                // Backspacing the empty draft out closes the `w` line, back
                // to the modal's first key.
                if draft.is_empty() {
                    prompt.draft = None;
                }
                prompt.error = None;
            }
            None
        }
        KeyCode::Enter => {
            if let Some(draft) = prompt.draft.as_deref() {
                let dir = draft.trim().to_owned();
                if dir.is_empty() {
                    prompt.error =
                        Some("`w` names a directory: type it after `w`, then Enter".into());
                    return None;
                }
                return match session_trust::resolve_trusted_dir(std::path::Path::new(&dir)) {
                    Ok(resolved) => Some(TrustResolution::Workspace(resolved)),
                    Err(error) => {
                        prompt.error = Some(error);
                        None
                    }
                };
            }
            None
        }
        KeyCode::Char('t') | KeyCode::Char('T') if !typing => Some(TrustResolution::TrustCwd),
        KeyCode::Char('c') | KeyCode::Char('C') if !typing => {
            Some(TrustResolution::ContinueUnbound)
        }
        KeyCode::Char('w') | KeyCode::Char('W') if !typing => {
            prompt.draft = Some(String::new());
            prompt.error = None;
            None
        }
        KeyCode::Char(c) if typing => {
            if let Some(draft) = prompt.draft.as_mut() {
                draft.push(c);
            }
            prompt.error = None;
            None
        }
        _ => None,
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interactive::tui::application::tests_support::idle_app;
    use crate::interactive::tui::types::TrustPrompt;

    /// `t` trusts the launch cwd: the modal closes and the stashed answer
    /// is the canonical cwd — exactly the named directory, never a parent.
    #[test]
    fn t_trusts_the_launch_cwd() {
        let mut app = idle_app();
        app.overlays.trust = Some(TrustPrompt::default());
        let dir = app
            .answer_trust(KeyCode::Char('t'), KeyModifiers::NONE)
            .expect("t answers");
        assert!(app.overlays.trust.is_none(), "the modal closes");
        assert_eq!(
            app.take_trust_answer().as_deref(),
            Some(dir.as_path()),
            "the answer drains exactly once"
        );
        assert!(
            app.take_trust_answer().is_none(),
            "the drain is single-shot"
        );
        let cwd = std::env::current_dir().unwrap();
        let canonical = std::fs::canonicalize(&cwd).unwrap_or(cwd);
        assert_eq!(dir, canonical, "trust binds exactly the launch cwd");
        let echo = app.transcript.blocks().last().expect("the echo is said");
        assert!(
            echo.text.contains("this session only"),
            "the echo states the session-only shape: {}",
            echo.text
        );
    }

    /// `c` (and Esc) continue unbound: the modal closes, nothing stashes,
    /// nothing binds — today's shape.
    #[test]
    fn c_and_esc_continue_unbound() {
        for code in [KeyCode::Char('c'), KeyCode::Char('C'), KeyCode::Esc] {
            let mut app = idle_app();
            app.overlays.trust = Some(TrustPrompt::default());
            assert!(
                app.answer_trust(code, KeyModifiers::NONE).is_none(),
                "continue binds nothing: {code:?}"
            );
            assert!(app.overlays.trust.is_none(), "the modal closes: {code:?}");
            assert!(
                app.take_trust_answer().is_none(),
                "nothing stashes for the loop: {code:?}"
            );
        }
    }

    /// The `w <dir>` line types inline and commits on Enter: the modal
    /// closes with exactly the typed directory, canonicalised.
    #[test]
    fn w_names_a_directory_inline() {
        let mut app = idle_app();
        app.overlays.trust = Some(TrustPrompt::default());
        assert!(
            app.answer_trust(KeyCode::Char('w'), KeyModifiers::NONE)
                .is_none(),
            "w opens the draft, answers nothing"
        );
        assert!(app.overlays.trust.is_some(), "the modal stays while typing");
        let dir = std::env::temp_dir();
        let text = dir.display().to_string();
        for c in text.chars() {
            assert!(
                app.answer_trust(KeyCode::Char(c), KeyModifiers::NONE)
                    .is_none(),
                "typing answers nothing"
            );
        }
        let resolved = app
            .answer_trust(KeyCode::Enter, KeyModifiers::NONE)
            .expect("Enter commits the typed dir");
        assert_eq!(
            resolved,
            std::fs::canonicalize(&dir).unwrap(),
            "the modal binds exactly the typed dir"
        );
        assert!(app.overlays.trust.is_none(), "the modal closes");
    }

    /// A bad directory refuses inline: the modal stays, with the error —
    /// never a launch failure, never a silent unbound session.
    #[test]
    fn a_bad_directory_refuses_inline_and_the_modal_stays() {
        let mut app = idle_app();
        app.overlays.trust = Some(TrustPrompt::default());
        assert!(
            app.answer_trust(KeyCode::Char('w'), KeyModifiers::NONE)
                .is_none()
        );
        for c in "/definitely/not/a/saya/dir".chars() {
            assert!(
                app.answer_trust(KeyCode::Char(c), KeyModifiers::NONE)
                    .is_none()
            );
        }
        assert!(
            app.answer_trust(KeyCode::Enter, KeyModifiers::NONE)
                .is_none(),
            "a bad dir answers nothing"
        );
        let prompt = app.overlays.trust.as_ref().expect("the modal stays");
        assert!(
            prompt.error.as_deref().is_some_and(|e| !e.is_empty()),
            "the refusal is said inline"
        );
        assert!(app.take_trust_answer().is_none(), "nothing binds");
    }

    /// Esc while typing cancels the `w` line back to the modal's first key
    /// — it does not continue unbound; a second Esc does that.
    #[test]
    fn esc_while_typing_cancels_the_draft_first() {
        let mut app = idle_app();
        app.overlays.trust = Some(TrustPrompt::default());
        assert!(
            app.answer_trust(KeyCode::Char('w'), KeyModifiers::NONE)
                .is_none()
        );
        assert!(
            app.answer_trust(KeyCode::Char('x'), KeyModifiers::NONE)
                .is_none()
        );
        assert!(
            app.answer_trust(KeyCode::Esc, KeyModifiers::NONE).is_none(),
            "the first Esc cancels the draft, answers nothing"
        );
        assert!(app.overlays.trust.is_some(), "the modal stays");
        assert!(
            app.answer_trust(KeyCode::Esc, KeyModifiers::NONE).is_none(),
            "the second Esc continues unbound"
        );
        assert!(app.overlays.trust.is_none(), "the modal closes");
    }
}
