//! The generation a readiness view reports, and what it says when the walk
//! behind it gives up.
//!
//! Readiness is advisory, so a source-generation walk that cannot finish leaves
//! the view `unknown` instead of failing the read that asked for it. It is not
//! silent, though: a walk that gave up because entries kept leaving the working
//! checkout — the class a live checkout produces, and the one the sandbox
//! probe's own cleanup produces inside it — is reported with its error class and
//! nothing else. Every other refusal keeps the handling it had.

use saya_harness::coding::git::IgnoredPaths;
use saya_harness::coding::integration::{CandidateError, DeltaError};
use saya_harness::coding::worktree::VerifiedCheckout;

use super::{CodingError, NativeCodingJob, prepare::SourceGeneration};

/// The generation of an already-verified W, or `None` when the walk refused.
/// The verification is the one the caller already holds for the delta, so the
/// binding is not paid twice.
pub(super) fn generation(
    job: &NativeCodingJob,
    verified: Option<(&VerifiedCheckout, &IgnoredPaths)>,
) -> Option<SourceGeneration> {
    let (verified, ignored) = verified?;
    reported(job.source_generation_from(verified, ignored))
}

/// The generation, or `None` when the walk refused. A refusal caused by an
/// entry that vanished under the walk is warned about; the view it feeds stays
/// `unknown`, which is what it was when every refusal was swallowed.
pub(super) fn reported(result: Result<SourceGeneration, CodingError>) -> Option<SourceGeneration> {
    match result {
        Ok(generation) => Some(generation),
        Err(error) => {
            if let Some(class) = vanished_class(&error) {
                tracing::warn!(
                    class,
                    "the source-generation walk gave up on an entry that vanished; readiness stays unknown"
                );
            }
            None
        }
    }
}

/// The error class of a walk that gave up on an entry that left the checkout,
/// or `None` for every other refusal. The class is a fixed name: no path, no
/// digest and no secret travels with it.
fn vanished_class(error: &CodingError) -> Option<&'static str> {
    let mut source = Some(error as &(dyn std::error::Error + 'static));
    while let Some(current) = source {
        if let Some(candidate) = current.downcast_ref::<CandidateError>()
            && matches!(candidate, CandidateError::Walk(DeltaError::Vanished))
        {
            return Some("vanished");
        }
        source = current.source();
    }
    None
}

#[cfg(all(test, unix))]
#[path = "readiness_generation_tests.rs"]
mod tests;
