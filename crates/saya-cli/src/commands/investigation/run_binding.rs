//! The review-binding decisions for `investigation run` (S7, invariant 3
//! and 4): the pure staleness check over the on-disk binding, the definition
//! revision, the resolved target, and the current combined schema
//! fingerprint — plus the post-success refresh that rewrites the binding.
//! Staleness never touches I/O, so the run path cannot execute a query the
//! review does not cover.
//!
//! A binding goes stale when any reviewed fact differs from the current
//! state: the definition's revision, the target profile (by name or by
//! opaque identity), or the recorded schema fingerprint. An absent binding
//! is never stale — the first explicit `--connection` mapping establishes
//! the review; it is written on success, never before.

use crate::commands::output::emit;
use crate::render::{RenderFormat, TerminalEvent};
use saya_store::{InvestigationRepository, LocalBinding};
use saya_types::investigation::InvestigationDefinitionV1;

/// Which reviewed fact no longer matches; the run refusal names each one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StaleReason {
    Revision,
    Target,
    Schema,
}

/// The staleness verdict: every reason the binding no longer covers the
/// current run, in a stable order; empty means the review still holds.
pub(super) fn staleness(
    binding: Option<&LocalBinding>,
    definition: &InvestigationDefinitionV1,
    target_profile: &str,
    current_identity: &str,
    current_fingerprint: Option<&str>,
) -> Vec<StaleReason> {
    let Some(binding) = binding else {
        return Vec::new();
    };
    let mut reasons = Vec::new();
    if binding.reviewed_revision != definition.revision {
        reasons.push(StaleReason::Revision);
    }
    if binding.profile != target_profile || binding.profile_identity != current_identity {
        reasons.push(StaleReason::Target);
    }
    if binding.reviewed_schema_fingerprint.as_deref() != current_fingerprint
        && binding.reviewed_schema_fingerprint.is_some()
    {
        reasons.push(StaleReason::Schema);
    }
    reasons
}

/// The refusal for a stale review: what changed and the way out.
pub(super) fn stale_message(reasons: &[StaleReason]) -> String {
    let named = reasons
        .iter()
        .map(|reason| match reason {
            StaleReason::Revision => "revision changed",
            StaleReason::Target => "target changed",
            StaleReason::Schema => "schema changed",
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("review is stale ({named}); pass --revalidate to re-review the current state")
}

/// The run-side review facts, as [`staleness`] and the refresh both read
/// them: the target profile, its opaque identity, and the combined schema
/// fingerprint of the referenced objects.
pub(super) struct Review {
    pub target: String,
    pub identity: String,
    pub fingerprint: Option<String>,
}

/// Writes the refreshed review binding after a successful execution (D4):
/// when `--revalidate` was passed or no binding existed, and also when the
/// existing binding predates fingerprinting — the save-time binding records
/// no schema fingerprint, so the first successful replay completes that
/// review. A fresh binding is left untouched, keeping its reviewed time
/// honest. A write failure is a visible warning, not a failed run: the
/// query succeeded and its output is already out.
pub(super) fn refresh_binding(
    repo: &InvestigationRepository,
    format: RenderFormat,
    definition: &InvestigationDefinitionV1,
    binding: Option<&LocalBinding>,
    revalidate: bool,
    review: &Review,
) {
    let completes_fingerprint = binding
        .is_some_and(|binding| binding.reviewed_schema_fingerprint.is_none())
        && review.fingerprint.is_some();
    if !(revalidate || binding.is_none() || completes_fingerprint) {
        return;
    }
    let refreshed = LocalBinding {
        version: LocalBinding::VERSION,
        id: definition.id.clone(),
        profile: review.target.clone(),
        profile_identity: review.identity.clone(),
        reviewed_revision: definition.revision,
        reviewed_schema_fingerprint: review.fingerprint.clone(),
        reviewed_unix_ms: unix_now_ms(),
    };
    if let Err(error) = repo.put_binding(&refreshed) {
        emit(
            TerminalEvent::Diagnostic {
                message: format!(
                    "the replay succeeded but the local review binding could not be written: {error}"
                ),
            },
            format,
        );
    }
}

/// Wall clock in unix milliseconds (0 before the epoch).
fn unix_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

#[cfg(test)]
#[path = "run_binding_tests.rs"]
mod tests;
