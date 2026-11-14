//! Refuses to re-run captured SQL when the alias now resolves elsewhere.
//!
//! A direct-SQL capture carries the executed source's opaque identity, but
//! the re-run entries (`--refresh`, `/chart`, bare `/explain`) resolve the
//! STORED label again at run time. When the alias was rebound to a different
//! endpoint/path meanwhile, the old SQL would silently run against the wrong
//! source — so the re-run is refused before any task spawns. Snapshots and
//! reports read the capture only and never consult this guard.

use super::super::capture::CapturedResult;
use super::super::transcript::{BlockKind, Transcript};
use super::super::types::LastQuery;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session::commands::SessionAction;
use crate::interactive::session::state::SessionState;
use crate::slash::ExportMode;
use saya_types::ExecutionEvidence;

/// Pure check: `Some(refusal)` ONLY on a proven mismatch — the evidence
/// carries an identity, its label equals the label being re-run, that label
/// still resolves, and the current resolution's identity differs. Unknown
/// stays unknown: `None` capture, `None` identity, a different label, or an
/// unresolvable label all return `None` (existing behavior/error unchanged).
/// The message names no row or SQL content.
pub(super) fn rerun_source_mismatch(
    captured: Option<&ExecutionEvidence>,
    label: &str,
    runtime: &RuntimeConfig,
) -> Option<String> {
    let evidence = captured?;
    let stored = evidence.connection_identity.as_deref()?;
    if evidence.connection_label != label {
        return None;
    }
    let resolved = runtime.named_profile(label).ok()?;
    let current = crate::profile_identity(label, resolved, &runtime.cache_scope);
    if current.as_str() == stored {
        return None;
    }
    Some(format!(
        "Refused to re-run the last query: it ran on '{label}', but '{label}' now resolves to a \
         different source. Run /sql again to capture a fresh result from the current source."
    ))
}

/// Pushes the pure refusal (when any) as an error block; `true` means the
/// caller must handle inline and spawn nothing.
pub(super) fn refuse_rerun(
    captured: &Option<CapturedResult>,
    label: &str,
    runtime: &RuntimeConfig,
    transcript: &mut Transcript,
) -> bool {
    let evidence = captured.as_ref().map(|capture| &capture.evidence);
    match rerun_source_mismatch(evidence, label, runtime) {
        Some(message) => {
            transcript.push(BlockKind::Error, message);
            true
        }
        None => false,
    }
}

/// The label a follow-up would resolve: the stored label wins, falling back
/// to the current profile. `None` means the action never re-runs a capture
/// (`--snapshot`, `/report`, explicit `/explain SQL`).
pub(super) fn rerun_label(
    action: &SessionAction,
    state: &SessionState,
    last_query: &Option<LastQuery>,
) -> Option<String> {
    match action {
        SessionAction::Export(request) if request.mode != Some(ExportMode::Snapshot) => {
            rerun_connection(last_query, state)
        }
        SessionAction::Chart(_) => rerun_connection(last_query, state),
        SessionAction::Explain(arg) if arg.trim().is_empty() => rerun_connection(last_query, state),
        _ => None,
    }
}

fn rerun_connection(last_query: &Option<LastQuery>, state: &SessionState) -> Option<String> {
    last_query
        .as_ref()
        .and_then(|lq| lq.connection.clone().or_else(|| state.profile.clone()))
}
