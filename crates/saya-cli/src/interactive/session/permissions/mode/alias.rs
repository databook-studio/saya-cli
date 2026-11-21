//! The consent axis's one-release spellings: `/approvals` and the launch's two
//! old flags.
//!
//! An alias is not a second command. `/approvals` parses into the same
//! [`ModeRequest`] `/mode` does and runs the same switch — the bypass gate and
//! the frozen-job refusal answer it identically — and the note that says it is
//! deprecated rides that one answer. The launch's `--approval-mode` and
//! `--permission-mode` are read into the same axis, so a flag and a command
//! cannot disagree about which posture is in force.
//!
//! Every note here is one line, and it is said once per command or launch.

use crate::agent::tools::shell_policy::ModeCell;
use crate::interactive::session::runtime::SessionRuntime;
use crate::interactive::session::state::SessionState;
use crate::slash::ModeRequest;
use saya_agent::PermissionMode;

use super::{BYPASS_REFUSAL, parse, switch_with_job};

/// The one-line note the deprecated `/approvals` spelling adds to the switch's
/// answer: the alias works for one release, and it is said once per command.
pub(crate) const APPROVALS_ALIAS_NOTE: &str = "`/approvals` is a deprecated alias of `/mode` and is \
     removed in the next release: use `/mode <value>`.";

/// The note the launch's deprecated `--approval-mode` spelling states on
/// stderr: accepted for one release, then removed.
pub(crate) const APPROVAL_MODE_FLAG_NOTE: &str = "note: `--approval-mode` is a deprecated alias of \
     `--mode` and is removed in the next release: use `--mode <value>`.";

/// The note the launch's deprecated `--permission-mode` spelling states on
/// stderr: accepted for one release, then removed.
pub(crate) const PERMISSION_MODE_FLAG_NOTE: &str = "note: `--permission-mode` is a deprecated alias \
     of `--mode` and is removed in the next release: use `--mode <value>`.";

/// The one switch every spelling of the consent axis runs: `/mode` and its
/// one-release `/approvals` alias both arrive here, so the gate, the journal
/// and the job adapter answer them identically.
///
/// A bare spelling is a read-back — the pure state layer answers it and never
/// reaches this — and the alias's deprecation note is appended to whatever the
/// switch answered, once.
pub(crate) fn switch_requested(
    state: &mut SessionState,
    session: &mut SessionRuntime,
    request: ModeRequest,
    job: Option<&ModeCell>,
) -> Result<String, String> {
    let Some(next) = request.value else {
        return Err("internal: a bare /mode is a read-back, never a switch".to_owned());
    };
    with_alias_note(switch_with_job(state, session, next, job), request)
}

/// Appends the deprecation note a deprecated spelling states, once — to an
/// answer or a refusal alike, so a refused alias still says it is an alias.
pub(crate) fn with_alias_note(
    result: Result<String, String>,
    request: ModeRequest,
) -> Result<String, String> {
    let Some(note) = request.via_approvals_alias.then_some(APPROVALS_ALIAS_NOTE) else {
        return result;
    };
    match result {
        Ok(mut message) => {
            message.push('\n');
            message.push_str(note);
            Ok(message)
        }
        Err(mut message) => {
            message.push('\n');
            message.push_str(note);
            Err(message)
        }
    }
}

/// The mode a launch states under `--mode` (or its two one-release alias
/// spellings), or the typed refusal: an unknown word, or `bypass` while this
/// process's configuration withholds it. The refusal is [`BYPASS_REFUSAL`] —
/// the switch's own wording — so the flag and `/mode` cannot refuse the same
/// request in two different sentences. `flag` is the spelling the launch used,
/// so the refusal names the flag the user typed.
pub(crate) fn stated_mode(
    flag: &str,
    spelling: &str,
    allow_bypass: bool,
) -> Result<PermissionMode, String> {
    let mode = parse(spelling).ok_or_else(|| {
        format!(
            "invalid {flag} `{spelling}`: expected ask, accept-edits, plan, bypass, read-only, \
             or never"
        )
    })?;
    if mode == PermissionMode::Bypass && !allow_bypass {
        return Err(BYPASS_REFUSAL.to_owned());
    }
    Ok(mode)
}
