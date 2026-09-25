//! One trust-modal key press resolved to an answer.

use super::super::types::TrustPrompt;
use crate::interactive::session_trust;
use ratatui::crossterm::event::KeyCode;

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
pub(crate) fn trust_key(prompt: &mut TrustPrompt, code: KeyCode) -> Option<TrustResolution> {
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
