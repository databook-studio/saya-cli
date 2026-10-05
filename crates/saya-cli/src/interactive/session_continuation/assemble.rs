//! Assembles the session continuation a completed compaction saves, and the
//! workspace digest resolver the compaction call sites inject.
//!
//! The facts come from the deterministic builder (observed tool activity plus
//! the persisted turns); the compaction summary becomes the generated
//! narrative only when it survives redaction unchanged; the existing user
//! notes ride across untouched. A record that cannot validate, or whose facts
//! are not redaction-stable, is refused entirely — the caller then leaves the
//! previous continuation exactly as it was.

use super::record::build_continuation_facts;
use crate::interactive::session_state::SessionState;
use saya_harness::workspace::Workspace;
use saya_store::StoredContinuation;
use saya_types::{
    CONTINUATION_VERSION, GeneratedNarrative, MAX_NARRATIVE_BYTES, NarrativeSource,
    SessionContinuationV1, redact,
};
use std::sync::Arc;

/// Builds the continuation a validated compaction saves: deterministic facts
/// from the session's observations, the summary as the narrative only when it
/// is redaction-stable and within the contract's prose bounds, and the
/// existing `Current` continuation's user notes preserved. `None` when the
/// record cannot be persisted safely — the caller leaves the previous
/// continuation untouched.
pub(crate) fn assemble_continuation(
    state: &SessionState,
    summary: &str,
    digest_of: &dyn Fn(&str) -> Option<String>,
) -> Option<SessionContinuationV1> {
    let facts = build_continuation_facts(
        &state.turns,
        state.workspace_root.as_deref(),
        state.profile.as_deref(),
        &state.observations,
        digest_of,
    );
    let mut record = SessionContinuationV1 {
        version: CONTINUATION_VERSION,
        source: facts.source,
        narrative: candidate_narrative(summary),
        user_notes: preserved_notes(state),
        files: facts.files,
        checks: facts.checks,
    };
    finalize(&mut record)?;
    Some(record)
}

/// The compaction summary as the generated narrative — but only when it can
/// be persisted exactly as spoken: non-empty, within the byte bound,
/// redaction-stable, and free of control characters beyond `\n`/`\t`.
/// Anything else is omitted and the facts still save. A sensitive narrative
/// is dropped here rather than rewritten on disk: a summary that shrinks
/// under the redactor is not what the model said, and a redaction that
/// lengthens the text could make every later save of the session fail.
fn candidate_narrative(summary: &str) -> Option<GeneratedNarrative> {
    let persistable = !summary.is_empty()
        && summary.len() <= MAX_NARRATIVE_BYTES
        && redact(summary) == summary
        && summary
            .chars()
            .all(|c| !c.is_control() || c == '\n' || c == '\t');
    persistable.then(|| GeneratedNarrative {
        text: summary.to_owned(),
        source: NarrativeSource::Summarizer,
    })
}

/// User-authored notes are preserved separately from the generated narrative:
/// they ride the previous typed continuation across a new compaction, and
/// never come from the summary.
fn preserved_notes(state: &SessionState) -> Option<String> {
    match &state.continuation {
        Some(StoredContinuation::Current(record)) => record.user_notes.clone(),
        Some(StoredContinuation::Unsupported(_)) | None => None,
    }
}

/// The last gate before handover: a check label the redactor would rewrite
/// means the observed facts cannot be persisted safely — the whole record is
/// refused; a narrative or notes the redactor would rewrite are dropped; and
/// the record must pass the contract's own validation.
fn finalize(record: &mut SessionContinuationV1) -> Option<()> {
    if record
        .checks
        .iter()
        .any(|check| redact(&check.label) != check.label)
    {
        return None;
    }
    if record
        .narrative
        .as_ref()
        .is_some_and(|narrative| redact(&narrative.text) != narrative.text)
    {
        record.narrative = None;
    }
    if record
        .user_notes
        .as_ref()
        .is_some_and(|notes| redact(notes) != *notes)
    {
        record.user_notes = None;
    }
    record.validate().is_ok().then_some(())
}

/// The whole-file digest resolver the compaction call sites inject: a
/// workspace-relative path resolves to the file's sha256 through the
/// contained read (the digest covers the whole file however the content read
/// is capped); no workspace, or an unreadable path, resolves to no digest.
pub(crate) fn digest_resolver(
    workspace: Option<Arc<Workspace>>,
) -> impl Fn(&str) -> Option<String> {
    move |rel: &str| {
        let workspace = workspace.as_ref()?;
        workspace.read(rel, 0).ok().map(|file| file.digest)
    }
}

#[cfg(test)]
#[path = "assemble_tests.rs"]
mod assemble_tests;
