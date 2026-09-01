//! Request-scoped log of the confirmed claims the turn's SQL contradicted —
//! the data the runtime turns into one [`saya_agent::AgentEvent::KnowledgeOverridden`]
//! after the agent loop.
//!
//! Collects only; nothing here is persisted, and nothing reaches a provider.
//! The log records an [`OverrideFindingDto`] exactly when the detector
//! ([`crate::contracts::detect_overrides`]) raises a finding for a statement
//! the tools executed; a statement that honours the claim, or that the
//! detector fails closed on (unparseable, partial, a join, an ambiguous
//! object), records nothing.
//!
//! Mirrors the [`super::propose::ProposedClaimsLog`] shape: a `Mutex<Vec<…>>`
//! the `&self` executor appends to and the owning runtime drains once. The
//! detector's findings are already bounded by the receipt's recall caps; the
//! cap here is defense-in-depth so a future caller cannot grow it unbounded.
//!
//! Findings are recorded per statement. A turn may run several statements that
//! contradict the same confirmed claim; [`drain`](OverrideLog::drain) dedupes
//! by `claim_id` keeping the first occurrence, so one event names a contradicted
//! claim once — the count of contradicting statements is not load-bearing for a
//! notice, and repeating the same claim would be noise.

use saya_agent::OverrideFindingDto;
use saya_connectors::sql_references;
use saya_types::SqlDialect;
use std::sync::{Arc, Mutex};

use crate::contracts::{OverrideFinding, RecallReceipt, detect_overrides};

use super::DatabaseTools;

/// Defense-in-depth: the detector's findings are already bounded by the
/// receipt's recall caps (≤ supplied claims). This stops a future caller from
/// growing the log unbounded across a many-statement turn.
const MAX_FINDINGS: usize = 64;

/// A request-scoped log of the override findings raised this turn.
pub(crate) struct OverrideLog {
    findings: Mutex<Vec<OverrideFindingDto>>,
}

impl OverrideLog {
    pub(crate) fn new() -> Self {
        Self {
            findings: Mutex::new(Vec::new()),
        }
    }

    /// Appends a finding for one statement. The detector bounds this by the
    /// receipt's caps; the cap here is defense-in-depth.
    pub(crate) fn record(&self, finding: OverrideFindingDto) {
        let mut guard = self.findings.lock().expect("override log not poisoned");
        if guard.len() < MAX_FINDINGS {
            guard.push(finding);
        }
    }

    /// Returns and clears the turn's findings, deduped by `claim_id` (first
    /// occurrence wins, in recording order) so one event names a contradicted
    /// claim once. A second call returns nothing — a turn cannot double-emit.
    pub(crate) fn drain(&self) -> Vec<OverrideFindingDto> {
        let all = std::mem::take(&mut *self.findings.lock().expect("override log not poisoned"));
        dedupe_by_claim(all)
    }
}

impl Default for OverrideLog {
    fn default() -> Self {
        Self::new()
    }
}

/// Maps one detector finding to its DTO. The finding's `kind` is a `&'static str`
/// the detector guarantees is `default_time_column`; the DTO owns a `String` so
/// it can cross the crate boundary into the event. No identity, no SQL.
pub(super) fn override_dto(finding: OverrideFinding) -> OverrideFindingDto {
    OverrideFindingDto {
        claim_id: finding.claim_id,
        kind: finding.kind.to_string(),
        claimed_value: finding.claimed_value,
        observed_columns: finding.observed_columns,
    }
}

impl DatabaseTools {
    /// Attaches the turn's recall receipt and the request-scoped override log
    /// the detector records into. Both are `None` in tests that do not
    /// exercise detection; the production runtime sets both from the assembled
    /// receipt and a fresh log it drains after the loop.
    pub(crate) fn with_recall_receipt(
        mut self,
        receipt: Option<Arc<RecallReceipt>>,
        override_log: Option<Arc<OverrideLog>>,
    ) -> Self {
        self.recall_receipt = receipt;
        self.override_log = override_log;
        self
    }

    /// Runs the override detector for one statement and records any findings
    /// into the turn's override log. Called from the query-tool
    /// dispatch, where the statement text and dialect are known. Independent
    /// of the observation log: detection works with `learning = off`, because a
    /// confirmed claim being contradicted is a fact about the turn regardless of
    /// whether learning is configured to collect evidence. Best-effort: a missing
    /// receipt or log records nothing, and a detector failure (unparseable SQL,
    /// a partial column list, a join, an ambiguous object) records nothing
    /// rather than guess — the detector's own fail-closed direction.
    pub(in crate::agent::tools) fn detect_and_record_overrides(
        &self,
        sql: &str,
        dialect: SqlDialect,
    ) {
        let (Some(receipt), Some(log)) = (&self.recall_receipt, &self.override_log) else {
            return;
        };
        let refs = sql_references(sql, dialect);
        for finding in detect_overrides(receipt, refs.as_ref()) {
            log.record(override_dto(finding));
        }
    }
}

/// Keeps the first occurrence of each `claim_id`, preserving order. A claim
/// contradicted in several statements is reported once.
fn dedupe_by_claim(findings: Vec<OverrideFindingDto>) -> Vec<OverrideFindingDto> {
    let mut seen: Vec<String> = Vec::with_capacity(findings.len());
    let mut kept = Vec::with_capacity(findings.len());
    for finding in findings {
        let key = finding.claim_id.as_str().to_string();
        if seen.iter().any(|s| s == &key) {
            continue;
        }
        seen.push(key);
        kept.push(finding);
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(id: &str, observed: &str) -> OverrideFindingDto {
        OverrideFindingDto {
            claim_id: ClaimId::parse(id).unwrap(),
            kind: "default_time_column".into(),
            claimed_value: "return_date".into(),
            observed_columns: vec![observed.into()],
        }
    }

    use saya_types::ClaimId;

    #[test]
    fn drain_returns_findings_in_order_and_clears() {
        let log = OverrideLog::new();
        log.record(finding("c-a", "rental_date"));
        log.record(finding("c-b", "checkout_date"));
        let drained = log.drain();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].claim_id.as_str(), "c-a");
        assert_eq!(drained[1].claim_id.as_str(), "c-b");
        // A second drain is empty — a turn cannot double-emit.
        assert!(log.drain().is_empty());
    }

    /// A claim contradicted in several statements is reported once.
    #[test]
    fn drain_dedupes_by_claim_id_keeping_the_first() {
        let log = OverrideLog::new();
        log.record(finding("c-a", "rental_date"));
        log.record(finding("c-b", "checkout_date"));
        // Same claim, second statement — dropped by the dedupe.
        log.record(finding("c-a", "checkout_date"));
        let drained = log.drain();
        assert_eq!(drained.len(), 2, "one finding per contradicted claim");
        assert_eq!(drained[0].claim_id.as_str(), "c-a");
        // The first occurrence's observed columns are kept.
        assert_eq!(drained[0].observed_columns, vec!["rental_date".to_string()]);
        assert_eq!(drained[1].claim_id.as_str(), "c-b");
    }

    /// The log never grows past the defense-in-depth cap.
    #[test]
    fn the_log_caps_at_max_findings() {
        let log = OverrideLog::new();
        for i in 0..(MAX_FINDINGS + 5) {
            log.record(finding(&format!("c-{i}"), "rental_date"));
        }
        assert_eq!(log.drain().len(), MAX_FINDINGS, "the log honors its cap");
    }
}
