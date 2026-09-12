//! The resume's grant derivation: the journal is the authority a resume
//! re-grants from, never `spec.json`.
//!
//! `PlanApproved` carries the approved scopes as the `--allow` grammar's
//! words — the payload the journal records before anything runs, append-only
//! and redacted at write (the interpreter approval's design §4). A resume
//! parses exactly those tokens back into capabilities, so a resumed run
//! carries precisely the capabilities the original approval carried: no
//! more, no fewer, and nothing a file edited between invocations could add.
//!
//! A journal written before the payload existed states no scopes. For those
//! runs the persisted spec stands in — the documented fallback — minus the
//! interpreter family: no journal ever granted an interpreter (the family
//! did not exist when the payload was absent), so a `spec.json` edited after
//! the fact cannot grant one either. The journal's silence on the
//! interpreter is a refusal, not an invitation.

use std::path::Path;

use saya_harness::journal::Journal;
use saya_types::{Capabilities, RunEvent};

/// The capabilities a resume re-grants: the journal's `PlanApproved` payload
/// parsed back through the `--allow` grammar when it states scopes; the
/// persisted spec, stripped of the interpreter family, when it does not.
pub(super) fn journal_grants(
    dir: &Path,
    spec_scopes: &Capabilities,
) -> Result<Capabilities, String> {
    let events = Journal::open(dir)
        .read()
        .map_err(|error| format!("run journal could not be read: {error}"))?;
    let Some(RunEvent::PlanApproved { scopes }) = events
        .iter()
        .rev()
        .find(|event| matches!(event, RunEvent::PlanApproved { .. }))
    else {
        // No approval on record: the engine's resume contract refuses this
        // run as unapproved. Until then, the spec stands in, stripped.
        return Ok(stripped_of_interpreters(spec_scopes));
    };
    if scopes.is_empty() {
        // The payload's absence is "scopes unstated here": the journal
        // predates the field, so the spec stands in — minus the interpreter
        // family, which only a journal could have granted.
        return Ok(stripped_of_interpreters(spec_scopes));
    }
    let approved = super::scopes::parse(scopes)
        .map_err(|error| format!("the journal's approved scopes do not parse: {error}"))?;
    Ok(approved.capabilities)
}

fn stripped_of_interpreters(scopes: &Capabilities) -> Capabilities {
    let mut scopes = scopes.clone();
    scopes.interpreter = None;
    scopes
}
