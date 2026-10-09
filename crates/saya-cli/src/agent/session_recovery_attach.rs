//! Attaches the session's checkpoint recorder to the per-turn
//! `DatabaseTools` build: the recovery parts ride the session universe's
//! handle, exposed only where a workspace is bound. The one-shot `ask`
//! path (`None`) and every non-session construction attach nothing; with
//! the flag off — the default — behaviour is byte-identical to today.

use crate::agent::tools::DatabaseTools;
use crate::interactive::session::universe::SessionUniverse;

/// Attaches the session's recovery recorder when the session has one to
/// give: a bound workspace and a recovery handle. `None` (the `ask` path)
/// or an unbound session is a no-op; the recorder itself gates every
/// capture on the session's flag at edit time, so flipping it mid-session
/// reaches the already-attached tools.
pub(crate) fn attach_recovery(
    tools: DatabaseTools,
    session: Option<&SessionUniverse>,
) -> DatabaseTools {
    tools.with_recovery(session.and_then(|s| s.recovery_handle()).map(|h| h.parts()))
}

#[cfg(test)]
#[path = "session_recovery_attach_tests.rs"]
mod tests;
