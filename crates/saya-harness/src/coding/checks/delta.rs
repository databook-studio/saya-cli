use super::{CheckReceipt, Diagnostic, DiagnosticSet};
use saya_types::{CodingCheckOutcome, Sha256Digest};
use std::cmp::Ordering;

/// Stable multiset delta between two bounded parser results. Each vector holds
/// at most the parser's 128 diagnostics; callers still bound retained deltas.
#[derive(Debug)]
pub struct DiagnosticDelta {
    introduced: Vec<Diagnostic>,
    unchanged: Vec<Diagnostic>,
    resolved: Vec<Diagnostic>,
    baseline_source: Sha256Digest,
    current_source: Sha256Digest,
    baseline_outcome: CodingCheckOutcome,
    current_outcome: CodingCheckOutcome,
    current_successful: bool,
}

impl DiagnosticDelta {
    pub(super) fn between(
        previous: &DiagnosticSet,
        current: &DiagnosticSet,
        old_receipt: &CheckReceipt,
        current_receipt: &CheckReceipt,
    ) -> Self {
        Self::between_sets(
            previous,
            current,
            old_receipt.wire_receipt().source_sha256().clone(),
            current_receipt.wire_receipt().source_sha256().clone(),
            old_receipt.outcome(),
            current_receipt,
        )
    }

    pub(super) fn between_persisted(
        previous: &DiagnosticSet,
        current: &DiagnosticSet,
        baseline_source: Sha256Digest,
        baseline_outcome: CodingCheckOutcome,
        current_receipt: &CheckReceipt,
    ) -> Self {
        Self::between_sets(
            previous,
            current,
            baseline_source,
            current_receipt.wire_receipt().source_sha256().clone(),
            baseline_outcome,
            current_receipt,
        )
    }

    fn between_sets(
        previous: &DiagnosticSet,
        current: &DiagnosticSet,
        baseline_source: Sha256Digest,
        current_source: Sha256Digest,
        baseline_outcome: CodingCheckOutcome,
        current_receipt: &CheckReceipt,
    ) -> Self {
        let mut introduced = Vec::new();
        let mut unchanged = Vec::new();
        let mut resolved = Vec::new();
        let mut old = previous.entries().iter().peekable();
        let mut new = current.entries().iter().peekable();
        // Parser results are sorted, retaining duplicates. Consume one entry
        // from each side for equality instead of collapsing counts into a set.
        while let (Some(left), Some(right)) = (old.peek(), new.peek()) {
            match left.cmp(right) {
                Ordering::Less => {
                    resolved.push((*left).clone());
                    old.next();
                }
                Ordering::Greater => {
                    introduced.push((*right).clone());
                    new.next();
                }
                Ordering::Equal => {
                    unchanged.push((*right).clone());
                    old.next();
                    new.next();
                }
            }
        }
        resolved.extend(old.cloned());
        introduced.extend(new.cloned());
        Self {
            introduced,
            unchanged,
            resolved,
            baseline_source,
            current_source,
            baseline_outcome,
            current_outcome: current_receipt.outcome(),
            current_successful: current_receipt.is_successful(),
        }
    }

    pub fn introduced(&self) -> &[Diagnostic] {
        &self.introduced
    }
    pub fn unchanged(&self) -> &[Diagnostic] {
        &self.unchanged
    }
    pub fn resolved(&self) -> &[Diagnostic] {
        &self.resolved
    }
    pub fn baseline_source(&self) -> &Sha256Digest {
        &self.baseline_source
    }
    pub fn current_source(&self) -> &Sha256Digest {
        &self.current_source
    }
    pub fn baseline_outcome(&self) -> CodingCheckOutcome {
        self.baseline_outcome
    }
    pub fn current_outcome(&self) -> CodingCheckOutcome {
        self.current_outcome
    }
    pub fn current_successful(&self) -> bool {
        self.current_successful
    }
}
