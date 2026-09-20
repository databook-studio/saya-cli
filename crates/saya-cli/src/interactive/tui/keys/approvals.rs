use ratatui::crossterm::event::KeyCode;
use saya_agent::ApprovalChoice;
/// Decides the answer for a key press aimed at a pending approval modal.
/// Enter is deliberately *not* an approval: the modal can appear while the
/// user is typing, and an implicit Enter must never allow SQL to run. Only an
/// explicit `y` (or `a`) approves once; `s` grants the offered session token
/// when one exists; `n`/`d`/Esc deny; anything else is left for the modal.
pub(crate) fn approval_answer(code: KeyCode) -> Option<bool> {
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
/// beside the habit keys, and Esc keeps denying.
pub(crate) fn approval_choice(code: KeyCode, grant: Option<&str>) -> Option<ApprovalChoice> {
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
