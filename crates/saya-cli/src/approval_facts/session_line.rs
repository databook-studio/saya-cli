//! The session-history line: the grant store's state for the call's family,
//! in the store's own words — the held tokens with the calls each has
//! answered, or the "no prior … grant" fact when the family is empty. The
//! tokens are the grammar's words verbatim; this module names families,
//! never re-spells tokens.

use saya_agent::SessionGrants;

use crate::grant_token::grant_family;

/// The session's prior grant state for the call's family. `None` when the
/// call suggests no token (the answers line already says so) or no store is
/// available — history is shown where it exists, never invented.
pub(super) fn session_history_line(
    grant: Option<&str>,
    grants: Option<&SessionGrants>,
) -> Option<String> {
    let token = grant?;
    let family = grant_family(token)?;
    let grants = grants?;
    let held: Vec<(String, u64)> = grants
        .tokens()
        .into_iter()
        .filter(|token| grant_family(token) == Some(family))
        .map(|token| {
            let calls = grants.calls(&token);
            (token, calls)
        })
        .collect();
    if held.is_empty() {
        return Some(format!("  session: no prior {family} grant"));
    }
    let joined = held
        .iter()
        .map(|(token, calls)| format!("{token} granted ({calls} calls)"))
        .collect::<Vec<_>>()
        .join(" · ");
    Some(format!("  session: {joined}"))
}
