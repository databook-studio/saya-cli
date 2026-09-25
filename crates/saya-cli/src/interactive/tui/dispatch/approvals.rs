//! `/approvals` follow-up: when the parsed line set a mode, say the bypass
//! activation line where the mode is set, and journal a fresh bypass
//! activation (a failed journal write is said, not silent).

use super::super::transcript::{BlockKind, Transcript};
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session_runtime::SessionRuntime;
use crate::interactive::session_state::SessionState;

pub(super) fn report_approvals_set(
    approvals_set: bool,
    before_mode: &str,
    transcript: &mut Transcript,
    state: &mut SessionState,
    runtime: &RuntimeConfig,
    session: &mut SessionRuntime,
) {
    if approvals_set {
        if let Some(activation) = crate::interactive::session_activation::line_if_bypass(
            state,
            runtime,
            &session.universe(),
        ) {
            transcript.push(BlockKind::System, activation);
        }
        // `before_mode` arrives as `&String` from the caller; deref once here
        // so the call below takes `&str` without a needless borrow at the
        // original call site.
        let before_mode: &str = before_mode;
        if crate::interactive::session_activation::bypass_activated_by_command(
            before_mode,
            &state.approval_mode,
        ) && let Err(error) = session
            .journal()
            .bypass_activated(saya_store::BypassSource::Command)
        {
            transcript.push(
                BlockKind::Error,
                crate::interactive::session_grants::journal_warning(&error),
            );
        }
    }
}
