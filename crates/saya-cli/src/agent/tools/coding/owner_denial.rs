//! The closed vocabulary of direct-root write denials.
//!
//! Every refusal the owner's write surface produces carries one of these
//! classes, the same fixed reason sentence each time, and one actionable next
//! step, so the model can change approach instead of retrying the same call.
//! Nothing here reads file contents or tool output: the words are fixed and
//! the only variable is a live helper's name.

use std::collections::BTreeMap;
use std::sync::Mutex;

/// One closed class of owner write denial.
///
/// There is no job-level write-path ceiling: the root may change any file in
/// its checkout, and no path lease is shared with a helper. What remains is the
/// toolset that carries no checkout at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OwnerWriteDenial {
    /// This toolset carries no working checkout, so it approves no write path.
    NoWritePath,
}

impl OwnerWriteDenial {
    /// The stable class name: closed vocabulary, safe to persist and compare.
    pub(crate) fn class(&self) -> &'static str {
        match self {
            Self::NoWritePath => "no_write_path_approved",
        }
    }

    /// The closed reason sentence for this class.
    fn reason(&self) -> String {
        match self {
            Self::NoWritePath => "this toolset holds no working checkout".to_owned(),
        }
    }

    /// The one fixed actionable next step for this class.
    pub(crate) fn next_step(&self) -> &'static str {
        match self {
            Self::NoWritePath => {
                "delegate the change to a helper, which writes the working checkout"
            }
        }
    }

    /// The whole denial sentence: the closed reason and the fixed next step.
    pub(crate) fn message(&self) -> String {
        format!(
            "owner write denied: {}; {}.",
            self.reason(),
            self.next_step()
        )
    }
}

/// The denial of the same class that first collapses to the next step.
const REPEAT_AFTER: u32 = 2;

/// The run-scoped tally that turns a repeated denial into one bounded notice.
/// Counting is advisory — it changes wording, never authority — so a poisoned
/// tally still words the denial, just without a notice.
#[derive(Debug, Default)]
pub(crate) struct DenialTally {
    counts: Mutex<BTreeMap<&'static str, u32>>,
}

impl DenialTally {
    /// Counts one denial of `class`.
    pub(crate) fn note(&self, class: &'static str) {
        if let Ok(mut counts) = self.counts.lock() {
            let entry = counts.entry(class).or_insert(0);
            *entry = entry.saturating_add(1);
        }
    }

    /// The first denial of a class is the closed teaching sentence. A later
    /// denial of the same class in this run collapses to the class, the count
    /// and the next step, so the root sees one closed reason instead of the
    /// same sentence again.
    pub(crate) fn wording(&self, denial: &OwnerWriteDenial) -> String {
        let count = self
            .counts
            .lock()
            .ok()
            .and_then(|counts| counts.get(denial.class()).copied())
            .unwrap_or(0);
        if count >= REPEAT_AFTER {
            return format!(
                "repeated denials ({} ×{}): {}.",
                denial.class(),
                count,
                denial.next_step()
            );
        }
        denial.message()
    }
}

#[cfg(test)]
#[path = "owner_denial_tests.rs"]
mod owner_denial_tests;
