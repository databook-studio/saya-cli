use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use saya_agent::ApprovalChoice;
/// A held Ctrl, Alt or Super makes the press an editing or global chord,
/// never an answer: the modal can appear mid-typing, and start-of-line
/// Ctrl+A or EOF Ctrl+D must never read as consent or denial. Shift stays
/// allowed — `Y`/`N` with shift are the answers their bare forms are.
fn modifier_held(mods: KeyModifiers) -> bool {
    mods.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
}

/// Decides the answer for a key press aimed at a pending approval modal.
/// Enter is deliberately *not* an approval: the modal can appear while the
/// user is typing, and an implicit Enter must never allow SQL to run. Only an
/// explicit `y` (or `a`) approves once; `s` grants the offered session token
/// when one exists; `n`/`d`/Esc deny; anything else is left for the modal.
/// A held Ctrl/Alt/Super is never an answer at all.
pub(crate) fn approval_answer(code: KeyCode, mods: KeyModifiers) -> Option<bool> {
    if modifier_held(mods) {
        return None;
    }
    match code {
        KeyCode::Char('y') | KeyCode::Char('Y') => Some(true),
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Some(false),
        _ => None,
    }
}

/// Decides the answer for a key press aimed at the tool-approval modal, where
/// a session grant can be offered. The `[s]` key grants exactly the token the
/// modal offered; with no token offered, `s` is not an answer at all (the
/// modal stays — it never offered a grant to take). `a` keeps allow-once
/// beside the habit keys, and Esc keeps denying. A held Ctrl/Alt/Super is
/// never an answer at all, so a pending modal cannot silently take an
/// editing chord — including a session grant.
pub(crate) fn approval_choice(
    code: KeyCode,
    mods: KeyModifiers,
    grant: Option<&str>,
) -> Option<ApprovalChoice> {
    if modifier_held(mods) {
        return None;
    }
    match code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Char('a') | KeyCode::Char('A') => {
            Some(ApprovalChoice::AllowOnce)
        }
        KeyCode::Char('s') | KeyCode::Char('S') => {
            grant.map(|token| ApprovalChoice::AllowSession {
                token: token.to_owned(),
            })
        }
        KeyCode::Char('n')
        | KeyCode::Char('N')
        | KeyCode::Char('d')
        | KeyCode::Char('D')
        | KeyCode::Esc => Some(ApprovalChoice::Deny),
        _ => None,
    }
}
