//! The resume's grant derivation: the journal is the authority a resume
//! re-grants from, never `spec.json`.
//!
//! `PlanApproved` carries the approved scopes as the `--allow` grammar's
//! words — the payload the journal records before anything runs, append-only
//! and redacted at write (the interpreter approval's design §4). A resume
//! parses exactly those tokens back into capabilities, so a resumed run
//! carries precisely the capabilities the original approval carried: no
//! more, no fewer, and nothing a file edited between invocations could add.
//! That includes the approval that stated nothing: a `--allow none` run
//! journals the empty set explicitly (`Some(vec![])`), and its resume
//! re-grants exactly that — an edited `spec.json` adds nothing to it. The
//! payload's words are also the resumed run's decider seeds — the frozen
//! policy (U4) holds exactly the scopes the journal states, never the
//! spec's.
//!
//! A journal written before the payload existed carries no field (`None`):
//! for those runs alone the persisted spec stands in — the documented
//! fallback — minus the interpreter family: no journal ever granted an
//! interpreter (the family did not exist when the payload was absent), so a
//! `spec.json` edited after the fact cannot grant one either. The journal's
//! silence there is a refusal, not an invitation. The fallback states no
//! seeds: only a journal's own words seed the resumed decider, and no
//! pre-payload journal could have stated a grant word.

use std::path::Path;

use saya_harness::journal::Journal;
use saya_types::{Capabilities, RunEvent};

/// What a resume re-grants: the journal's `PlanApproved` payload parsed back
/// through the `--allow` grammar — its capabilities for the plan gate and
/// its words for the frozen decider's seeds — or the persisted spec, stripped
/// of the interpreter family and stated without seeds, when the payload
/// predates the field.
pub(super) struct JournalGrants {
    pub(super) capabilities: Capabilities,
    /// The journal's own scope words, exactly as stated — the resumed
    /// decider's seeds. Empty for a pre-payload journal: the spec fallback
    /// grants capabilities, never grant words.
    pub(super) tokens: Vec<String>,
}

/// The capabilities and seed words a resume re-grants: from the journal's
/// `PlanApproved` payload when it states scopes — including the stated
/// empty approval of a `--allow none` run, which re-grants nothing; from
/// the persisted spec, stripped of the interpreter family, only when the
/// journal predates the payload field.
pub(super) fn journal_grants(
    dir: &Path,
    spec_scopes: &Capabilities,
) -> Result<JournalGrants, String> {
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
        return Ok(JournalGrants {
            capabilities: stripped_of_interpreters(spec_scopes),
            tokens: Vec::new(),
        });
    };
    let Some(scopes) = scopes else {
        // The field's absence is "scopes unstated here": the journal
        // predates the payload, so the spec stands in — minus the
        // interpreter family, which only a journal could have granted —
        // and states no grant words: nothing a pre-payload journal could
        // have approved.
        return Ok(JournalGrants {
            capabilities: stripped_of_interpreters(spec_scopes),
            tokens: Vec::new(),
        });
    };
    // A stated payload — words or the empty set — is the whole of the
    // re-grant: the capability words re-parse on the run surface, and the
    // carried grant words (a `sql:<connection>` token) ride the seeds
    // verbatim, so the resumed run's decider consults exactly what the
    // journal approved. The empty payload parses to no capabilities and no
    // seeds: a `none` run's resume re-grants precisely nothing, whatever
    // the spec says.
    let approved = super::scopes::parse(scopes, super::scopes::Surface::Run)
        .map_err(|error| format!("the journal's approved scopes do not parse: {error}"))?;
    Ok(JournalGrants {
        capabilities: approved.capabilities,
        tokens: scopes.clone(),
    })
}

fn stripped_of_interpreters(scopes: &Capabilities) -> Capabilities {
    let mut scopes = scopes.clone();
    scopes.interpreter = None;
    scopes
}
