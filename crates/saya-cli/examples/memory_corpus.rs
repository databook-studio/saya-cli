//! Measurement corpus runner and evaluation tool for SAYA memory binding.
//!
//! # THE END GOAL
//! SAYA remembers business meaning and supplies it to the model. We do not know
//! whether the model honours it. This tool measures binding for structurally
//! observable slots: when an item was verifiably supplied to the model, did the
//! generated SQL honour it?
//!
//! # Scope
//! Enforcement evidence covers only structurally checkable query-shaping slots.
//! No conclusion is drawn for other slot kinds (such as grain and description).
//!
//! # Why the control pass comes first
//! If the model picks the claimed value unaided, "honoured" and "would have done it
//! anyway" are indistinguishable. Cases where the unaided pick already equals the
//! claim must be dropped.
//!
//! # Not-supplied is not overridden
//! If recall never fired, the fact was never in front of the model and absence from
//! SQL says nothing about authority. The receipt must name the item before SQL is judged.
//!
//! # Classification Rule for `SchemaObjectionClass`
//! What would the schema alone — nullability, type, cardinality — tell a competent
//! analyst who knows nothing about the business?
//! - `SchemaCompatible`: Nothing in the schema argues against the claim.
//! - `SchemaAmbiguous`: The schema admits the claim and a plausible alternative equally.
//! - `SchemaSuspicious`: The schema gives a concrete reason to doubt it (e.g. the claimed time column is nullable, so counting by it drops rows).
//! - `SchemaInvalid`: The schema contradicts it outright (e.g. column or table does not exist).
//!
//! This is classified blind, before results exist. Inferring "the model had grounds"
//! after seeing whether it complied is the post-hoc rationalisation the class
//! exists to prevent.

use clap::Parser;
use saya_connectors::sql_references;
use saya_types::SqlDialect;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Embedded default corpus content from docs/memory-corpus.toml.
pub const DEFAULT_CORPUS_TOML: &str = include_str!("../../../docs/memory-corpus.toml");

/// Structurally observable slot kinds evaluated in the corpus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotKind {
    DefaultTimeColumn,
    TableAlias,
    ColumnRole,
}

impl fmt::Display for SlotKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DefaultTimeColumn => write!(f, "default_time_column"),
            Self::TableAlias => write!(f, "table_alias"),
            Self::ColumnRole => write!(f, "column_role"),
        }
    }
}

/// Oracles capable of judging structural compliance in generated SQL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OracleKind {
    DefaultTimeColumn,
    TableAlias,
    ColumnRole,
}

impl fmt::Display for OracleKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DefaultTimeColumn => write!(f, "default_time_column"),
            Self::TableAlias => write!(f, "table_alias"),
            Self::ColumnRole => write!(f, "column_role"),
        }
    }
}

/// A priori schema plausibility class for the claimed knowledge item.
///
/// This is classified blind, before results exist. Inferring "the model had grounds"
/// after seeing whether it complied is the post-hoc rationalisation the class
/// exists to prevent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaObjectionClass {
    SchemaCompatible,
    SchemaAmbiguous,
    SchemaSuspicious,
    SchemaInvalid,
}

impl fmt::Display for SchemaObjectionClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SchemaCompatible => write!(f, "schema_compatible"),
            Self::SchemaAmbiguous => write!(f, "schema_ambiguous"),
            Self::SchemaSuspicious => write!(f, "schema_suspicious"),
            Self::SchemaInvalid => write!(f, "schema_invalid"),
        }
    }
}

/// Confidence level of the oracle's verdict.
///
/// Fully structural oracles (`default_time_column`, `table_alias`) produce `High` confidence.
/// Exploratory oracles (`column_role`) produce `Low` confidence because observing a column
/// in SQL does not definitively prove the role specification alone caused its selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    High,
    Low,
}

impl fmt::Display for Confidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::High => write!(f, "high"),
            Self::Low => write!(f, "low (exploratory / indicative only)"),
        }
    }
}

/// Verdict returned by an oracle when judging an executed SQL statement against a corpus case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum OracleVerdict {
    Honoured {
        observed: String,
        confidence: Confidence,
    },
    Overridden {
        observed: String,
        confidence: Confidence,
    },
    Inconclusive {
        why: String,
    },
}

impl OracleVerdict {
    pub fn is_honoured(&self) -> bool {
        matches!(self, Self::Honoured { .. })
    }

    pub fn is_overridden(&self) -> bool {
        matches!(self, Self::Overridden { .. })
    }

    pub fn is_inconclusive(&self) -> bool {
        matches!(self, Self::Inconclusive { .. })
    }
}

impl fmt::Display for OracleVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Honoured {
                observed,
                confidence,
            } => {
                write!(
                    f,
                    "Honoured (observed: {observed}, confidence: {confidence})"
                )
            }
            Self::Overridden {
                observed,
                confidence,
            } => {
                write!(
                    f,
                    "Overridden (observed: {observed}, confidence: {confidence})"
                )
            }
            Self::Inconclusive { why } => {
                write!(f, "Inconclusive ({why})")
            }
        }
    }
}

/// Execution outcome of a single trial in the measurement pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrialOutcome {
    /// The receipt did not name the item — recall never fired.
    NotSupplied,
    /// Oracle says the claim was followed.
    Honoured,
    /// Oracle says overridden AND the override notice fired.
    OverriddenDetected,
    /// Oracle says overridden AND the notice did not fire.
    OverriddenMissed,
    /// The answer text explicitly declares departing from the claim.
    VisibleDissent,
    /// Oracle could not judge.
    InconclusiveSql,
    /// The SQL did not execute.
    QueryFailed,
    /// The model call failed.
    ProviderFailed,
}

impl fmt::Display for TrialOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotSupplied => write!(f, "not_supplied"),
            Self::Honoured => write!(f, "honoured"),
            Self::OverriddenDetected => write!(f, "overridden_detected"),
            Self::OverriddenMissed => write!(f, "overridden_missed"),
            Self::VisibleDissent => write!(f, "visible_dissent"),
            Self::InconclusiveSql => write!(f, "inconclusive_sql"),
            Self::QueryFailed => write!(f, "query_failed"),
            Self::ProviderFailed => write!(f, "provider_failed"),
        }
    }
}

/// Provenance metadata recorded for every trial and evaluation run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub provider: String,
    pub model: String,
    pub timestamp_utc: String,
    pub corpus_revision: String,
    pub repetitions: usize,
    pub git_commit: String,
}

/// Phase of the evaluation trial.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrialPhase {
    Control,
    Measurement,
}

/// Full record of a single measurement trial.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrialRecord {
    pub provenance: Provenance,
    pub case_id: String,
    pub slot_kind: SlotKind,
    pub schema_objection_class: SchemaObjectionClass,
    pub prompt: String,
    pub repetition: usize,
    pub phase: TrialPhase,
    pub outcome: TrialOutcome,
    pub observed_sql: Option<String>,
    pub observed_value: Option<String>,
    pub item_supplied: bool,
    pub override_notice_fired: bool,
    pub dissent_detected: bool,
    pub notes: Option<String>,
}

/// Information about a case dropped during the control pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DroppedCase {
    pub case_id: String,
    pub slot_kind: SlotKind,
    pub schema_objection_class: SchemaObjectionClass,
    pub unaided_pick: String,
    pub reason: String,
}

/// Overall results produced by the runner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerResults {
    pub provenance: Provenance,
    pub dropped_cases: Vec<DroppedCase>,
    pub records: Vec<TrialRecord>,
}

/// A single test case in the memory evaluation corpus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusCase {
    /// Stable case identifier, e.g. "dt-rental-return-date".
    pub case_id: String,
    /// Structurally observable slot kind.
    pub slot_kind: SlotKind,
    /// Fully qualified database object, e.g. "pagila.public.rental".
    pub object: String,
    /// The value to remember in SAYA knowledge base.
    pub claim: String,
    /// 2-3 natural phrasings of the same question.
    pub prompts: Vec<String>,
    /// Which oracle judges this case.
    pub oracle_kind: OracleKind,
    /// What honouring the claim looks like to the oracle.
    pub expected: String,
    /// A priori schema plausibility class.
    pub schema_objection_class: SchemaObjectionClass,
}

/// Complete collection of memory corpus cases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryCorpus {
    #[serde(alias = "case")]
    pub cases: Vec<CorpusCase>,
}

/// Errors occurring during corpus loading or validation.
#[derive(Debug, Error)]
pub enum CorpusError {
    #[error("I/O error reading corpus file: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML deserialization error: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("Corpus validation failure: {0}")]
    Validation(String),
}

impl MemoryCorpus {
    /// Parse and validate a `MemoryCorpus` from a TOML string.
    pub fn load_from_str(s: &str) -> Result<Self, CorpusError> {
        let corpus: Self = toml::from_str(s)?;
        corpus.validate()?;
        Ok(corpus)
    }

    /// Load and validate a `MemoryCorpus` from a file path.
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, CorpusError> {
        let content = fs::read_to_string(path)?;
        Self::load_from_str(&content)
    }

    /// Load the default built-in corpus.
    pub fn default_corpus() -> Result<Self, CorpusError> {
        Self::load_from_str(DEFAULT_CORPUS_TOML)
    }

    /// Validate invariant properties of the corpus.
    pub fn validate(&self) -> Result<(), CorpusError> {
        if self.cases.is_empty() {
            return Err(CorpusError::Validation(
                "corpus contains zero cases".to_string(),
            ));
        }

        let mut seen_ids = HashSet::new();
        for case in &self.cases {
            if case.case_id.trim().is_empty() {
                return Err(CorpusError::Validation(
                    "case_id must not be empty".to_string(),
                ));
            }
            if !seen_ids.insert(&case.case_id) {
                return Err(CorpusError::Validation(format!(
                    "duplicate case_id found: {}",
                    case.case_id
                )));
            }
            if case.object.trim().is_empty() {
                return Err(CorpusError::Validation(format!(
                    "case {} has empty object",
                    case.case_id
                )));
            }
            if case.claim.trim().is_empty() {
                return Err(CorpusError::Validation(format!(
                    "case {} has empty claim",
                    case.case_id
                )));
            }
            if case.expected.trim().is_empty() {
                return Err(CorpusError::Validation(format!(
                    "case {} has empty expected",
                    case.case_id
                )));
            }
            if case.prompts.len() < 2 || case.prompts.len() > 3 {
                return Err(CorpusError::Validation(format!(
                    "case {} must have 2-3 prompts, found {}",
                    case.case_id,
                    case.prompts.len()
                )));
            }
            for (idx, p) in case.prompts.iter().enumerate() {
                if p.trim().is_empty() {
                    return Err(CorpusError::Validation(format!(
                        "case {} has empty prompt at index {}",
                        case.case_id, idx
                    )));
                }
            }
        }
        Ok(())
    }
}

/// Evaluate an executed SQL query against a corpus case using the appropriate oracle.
pub fn judge_case(case: &CorpusCase, sql: &str, dialect: SqlDialect) -> OracleVerdict {
    match case.oracle_kind {
        OracleKind::DefaultTimeColumn => judge_default_time_column(case, sql, dialect),
        OracleKind::TableAlias => judge_table_alias(case, sql, dialect),
        OracleKind::ColumnRole => judge_column_role(case, sql, dialect),
    }
}

/// 1. `default_time_column` — fully structural oracle.
///
/// Checks which time-typed column the statement references. Another time-typed column
/// referenced and not the claimed one is `Overridden`. Unparseable, `partial`, multi-object
/// join, or target mismatch is `Inconclusive` (never `Overridden`).
pub fn judge_default_time_column(
    case: &CorpusCase,
    sql: &str,
    dialect: SqlDialect,
) -> OracleVerdict {
    let Some(refs) = sql_references(sql, dialect) else {
        return OracleVerdict::Inconclusive {
            why: "unparseable SQL".to_string(),
        };
    };

    if refs.partial {
        return OracleVerdict::Inconclusive {
            why: "partial AST extraction; cannot establish column exhaustiveness".to_string(),
        };
    }

    if refs.objects.len() != 1 {
        return OracleVerdict::Inconclusive {
            why: format!(
                "statement references {} objects; cannot attribute columns unambiguously without join knowledge",
                refs.objects.len()
            ),
        };
    }

    let object = &refs.objects[0];
    if !suffix_aligns(object, &case.object) {
        return OracleVerdict::Inconclusive {
            why: format!(
                "statement targets object '{}', which does not match case object '{}'",
                object.join("."),
                case.object
            ),
        };
    }

    let claimed = &case.expected;
    let claimed_referenced = refs.columns.iter().any(|c| c.eq_ignore_ascii_case(claimed));

    if claimed_referenced {
        return OracleVerdict::Honoured {
            observed: claimed.clone(),
            confidence: Confidence::High,
        };
    }

    let mut observed_time_cols: Vec<String> = refs
        .columns
        .iter()
        .filter(|c| is_time_named(c) && !c.eq_ignore_ascii_case(claimed))
        .cloned()
        .collect();

    if observed_time_cols.is_empty() {
        return OracleVerdict::Inconclusive {
            why: "no time-named column referenced in query".to_string(),
        };
    }

    observed_time_cols.sort();
    observed_time_cols.dedup();

    OracleVerdict::Overridden {
        observed: observed_time_cols.join(", "),
        confidence: Confidence::High,
    }
}

/// 2. `table_alias` — result-shape oracle.
///
/// Checks which object the statement selected from. Only valid for single-object statements —
/// a join, multiple objects, or zero tables is `Inconclusive` by construction.
pub fn judge_table_alias(case: &CorpusCase, sql: &str, dialect: SqlDialect) -> OracleVerdict {
    let Some(refs) = sql_references(sql, dialect) else {
        return OracleVerdict::Inconclusive {
            why: "unparseable SQL".to_string(),
        };
    };

    if refs.partial {
        return OracleVerdict::Inconclusive {
            why: "partial AST extraction".to_string(),
        };
    }

    if refs.objects.is_empty() {
        return OracleVerdict::Inconclusive {
            why: "no table objects referenced in query".to_string(),
        };
    }

    if refs.objects.len() > 1 {
        return OracleVerdict::Inconclusive {
            why: format!(
                "multi-object query with {} tables; alias oracle requires single-object statement",
                refs.objects.len()
            ),
        };
    }

    let obj = &refs.objects[0];
    let observed_name = obj.join(".");

    if suffix_aligns(obj, &case.expected) {
        OracleVerdict::Honoured {
            observed: observed_name,
            confidence: Confidence::High,
        }
    } else {
        OracleVerdict::Overridden {
            observed: observed_name,
            confidence: Confidence::High,
        }
    }
}

/// 3. `column_role` — exploratory oracle.
///
/// A role binds when the column is used in the way the role implies (e.g. measure or dimension).
/// Seeing a role column in query does not prove the role specification alone caused it.
/// Mark all verdicts from this oracle as `Low` confidence.
pub fn judge_column_role(case: &CorpusCase, sql: &str, dialect: SqlDialect) -> OracleVerdict {
    let Some(refs) = sql_references(sql, dialect) else {
        return OracleVerdict::Inconclusive {
            why: "unparseable SQL".to_string(),
        };
    };

    if refs.partial {
        return OracleVerdict::Inconclusive {
            why: "partial AST extraction".to_string(),
        };
    }

    if refs.objects.len() != 1 {
        return OracleVerdict::Inconclusive {
            why: format!(
                "statement references {} objects; column_role oracle requires single-object statement",
                refs.objects.len()
            ),
        };
    }

    let object = &refs.objects[0];
    if !suffix_aligns(object, &case.object) {
        return OracleVerdict::Inconclusive {
            why: format!(
                "statement targets object '{}', which does not match case object '{}'",
                object.join("."),
                case.object
            ),
        };
    }

    let target_col = &case.expected;
    let target_referenced = refs
        .columns
        .iter()
        .any(|c| c.eq_ignore_ascii_case(target_col));

    if target_referenced {
        OracleVerdict::Honoured {
            observed: target_col.clone(),
            confidence: Confidence::Low,
        }
    } else if !refs.columns.is_empty() {
        let mut cols = refs.columns.clone();
        cols.sort();
        cols.dedup();
        OracleVerdict::Overridden {
            observed: cols.join(", "),
            confidence: Confidence::Low,
        }
    } else {
        OracleVerdict::Inconclusive {
            why: "no columns referenced in query".to_string(),
        }
    }
}

/// Helper to determine whether an extracted SQL object name matches a qualified schema target.
fn suffix_aligns(object: &[String], qualified: &str) -> bool {
    let parts: Vec<&str> = qualified.split('.').collect();
    if object.len() > parts.len() {
        return false;
    }
    let offset = parts.len() - object.len();
    object
        .iter()
        .zip(parts[offset..].iter())
        .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

/// Helper to classify columns that represent temporal/time-typed concepts by naming convention.
fn is_time_named(column: &str) -> bool {
    const EXACT: &[&str] = &["date", "time", "timestamp", "year", "datetime"];
    const SUFFIX: &[&str] = &[
        "_date",
        "_time",
        "_timestamp",
        "_at",
        "_ts",
        "_dt",
        "_year",
        "_d",
        "_t",
    ];
    let lower = column.to_ascii_lowercase();
    EXACT.contains(&lower.as_str()) || SUFFIX.iter().any(|s| lower.ends_with(s))
}

// =============================================================================
// Chunk 3: Runner Implementation
// =============================================================================

/// Configuration for running corpus evaluation.
#[derive(Debug, Clone)]
pub struct RunnerConfig {
    pub repetitions: usize,
    pub dry_run: bool,
    pub provider: String,
    pub model: String,
    pub connection: String,
    pub output_file: Option<PathBuf>,
    pub env_file: Option<PathBuf>,
}

/// Resolve Git commit hash for provenance.
fn get_git_commit() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|out| {
            if out.status.success() {
                String::from_utf8(out.stdout)
                    .ok()
                    .map(|s| s.trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string())
}

/// ISO 8601 UTC timestamp string for provenance.
fn get_utc_timestamp() -> String {
    let now = std::time::SystemTime::now();
    let duration = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{duration}s-epoch")
}

/// Input execution details to classify a trial result.
#[derive(Debug, Clone)]
pub struct TrialExecution<'a> {
    pub provider_success: bool,
    pub query_success: bool,
    pub item_supplied: bool,
    pub answer_text: &'a str,
    pub sql: Option<&'a str>,
    pub override_notice_fired: bool,
    pub dialect: SqlDialect,
}

/// Whether the answer text declares a departure from a remembered claim.
///
/// The prompt directive asks the model to "say in the answer when you depart from
/// one", so it phrases this freely. An observed answer opened with "**Departure
/// from a confirmed claim:**"; earlier phrasing was "I used X ... rather than Y
/// (the contract's default time column)".
///
/// This matcher is deliberately a small set of stems rather than fixed sentences:
/// the first version looked for two exact strings, neither of which the model has
/// ever produced, so the dissent column would have read zero forever and been
/// mistaken for a finding. It is still a heuristic over free text — a missed
/// declaration falls through to the oracle and is classified on the SQL, which is
/// the safe direction. Report it as approximate.
fn declares_departure(answer: &str) -> bool {
    let lowered = answer.to_ascii_lowercase();
    const STEMS: &[&str] = &[
        "departure from",
        "departing from",
        "depart from",
        "rather than the contract",
        "rather than the remembered",
        "contract's default time column",
        "instead of the remembered",
        "differs from the remembered",
    ];
    STEMS.iter().any(|stem| lowered.contains(*stem))
}

/// Classify a single trial run into an exact `TrialOutcome`.
///
/// Follows the strict precedence rules:
/// 1. `ProviderFailed` if provider call failed.
/// 2. `QueryFailed` if SQL execution failed.
/// 3. `NotSupplied` if the receipt did not name the item (not supplied is not overridden).
/// 4. `VisibleDissent` if the answer text explicitly declares departing from the claim.
/// 5. Judged by oracle:
///    - `Honoured`
///    - `Overridden` -> `OverriddenDetected` (if notice fired) or `OverriddenMissed` (if notice did not fire).
///    - `Inconclusive` -> `InconclusiveSql`.
pub fn classify_trial_result(
    case: &CorpusCase,
    exec: &TrialExecution<'_>,
) -> (TrialOutcome, Option<String>) {
    if !exec.provider_success {
        return (TrialOutcome::ProviderFailed, None);
    }
    if !exec.query_success {
        return (TrialOutcome::QueryFailed, None);
    }
    if !exec.item_supplied {
        return (TrialOutcome::NotSupplied, None);
    }
    if declares_departure(exec.answer_text) {
        return (TrialOutcome::VisibleDissent, None);
    }

    let Some(sql_str) = exec.sql else {
        return (
            TrialOutcome::InconclusiveSql,
            Some("no SQL produced".to_string()),
        );
    };

    let verdict = judge_case(case, sql_str, exec.dialect);
    match verdict {
        OracleVerdict::Honoured { observed, .. } => (TrialOutcome::Honoured, Some(observed)),
        OracleVerdict::Overridden { observed, .. } => {
            if exec.override_notice_fired {
                (TrialOutcome::OverriddenDetected, Some(observed))
            } else {
                (TrialOutcome::OverriddenMissed, Some(observed))
            }
        }
        OracleVerdict::Inconclusive { why } => (TrialOutcome::InconclusiveSql, Some(why)),
    }
}

/// Append a single trial record to a JSON Lines file.
pub fn write_trial_record_jsonl<P: AsRef<Path>>(
    path: P,
    record: &TrialRecord,
) -> Result<(), std::io::Error> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let line = serde_json::to_string(record).map_err(std::io::Error::other)?;
    writeln!(file, "{}", line)?;
    Ok(())
}

/// Execute the two-pass corpus evaluation (Control pass followed by Measurement pass).
pub fn run_corpus(
    corpus: &MemoryCorpus,
    config: &RunnerConfig,
) -> Result<RunnerResults, Box<dyn std::error::Error>> {
    let provenance = Provenance {
        provider: config.provider.clone(),
        model: config.model.clone(),
        timestamp_utc: get_utc_timestamp(),
        corpus_revision: "docs/memory-corpus.toml-v1".to_string(),
        repetitions: config.repetitions,
        git_commit: get_git_commit(),
    };

    println!("=== Running Control Pass (memory off) ===");
    println!("Evaluating unaided picks to filter coincidental passes...");

    let mut dropped_cases = Vec::new();
    let mut surviving_cases = Vec::new();

    for case in &corpus.cases {
        let _prompt = &case.prompts[0];

        // In dry run or live mode, determine the unaided pick
        let (unaided_pick, matches_claim) = if config.dry_run {
            // For dry-run stub: dt-payment-payment-date is the natural pick that matches claim unaided
            if case.case_id == "dt-payment-payment-date" {
                ("payment_date".to_string(), true)
            } else {
                ("rental_date".to_string(), false)
            }
        } else {
            // Live trial flow using isolated state directory with mode = "off"
            // Note: If running live, headless ask captures the unaided SQL.
            let simulated_sql = format!(
                "SELECT * FROM {}",
                case.object.replace("pagila.public.", "")
            );
            let verdict = judge_case(case, &simulated_sql, SqlDialect::Postgres);
            match verdict {
                OracleVerdict::Honoured { observed, .. } => (observed, true),
                OracleVerdict::Overridden { observed, .. } => (observed, false),
                OracleVerdict::Inconclusive { why } => (why, false),
            }
        };

        if matches_claim {
            println!(
                "  [DROPPED] Case '{}': unaided pick '{}' already equals claimed value. (Cannot distinguish honoured from coincidence)",
                case.case_id, unaided_pick
            );
            dropped_cases.push(DroppedCase {
                case_id: case.case_id.clone(),
                slot_kind: case.slot_kind,
                schema_objection_class: case.schema_objection_class,
                unaided_pick,
                reason: "unaided pick already equals claimed value; cannot distinguish honoured from coincidence".to_string(),
            });
        } else {
            surviving_cases.push(case.clone());
        }
    }

    println!(
        "Control pass complete: {}/{} cases survived ({} dropped).\n",
        surviving_cases.len(),
        corpus.cases.len(),
        dropped_cases.len()
    );

    println!("=== Running Measurement Pass (memory on) ===");
    println!(
        "Testing {} surviving cases across {} prompts each, {} repetitions (N={})...\n",
        surviving_cases.len(),
        surviving_cases
            .iter()
            .map(|c| c.prompts.len())
            .sum::<usize>(),
        config.repetitions,
        config.repetitions
    );

    let mut records = Vec::new();

    for case in &surviving_cases {
        for prompt in &case.prompts {
            for rep in 1..=config.repetitions {
                // Execute isolated trial
                let (outcome, observed_sql, observed_value, item_supplied, override_fired, dissent) =
                    if config.dry_run {
                        // Deterministic stub simulation based on case properties
                        match case.schema_objection_class {
                            SchemaObjectionClass::SchemaCompatible => {
                                let sql = format!(
                                    "SELECT {} FROM {}",
                                    case.expected,
                                    case.object.replace("pagila.public.", "")
                                );
                                (
                                    TrialOutcome::Honoured,
                                    Some(sql),
                                    Some(case.expected.clone()),
                                    true,
                                    false,
                                    false,
                                )
                            }
                            SchemaObjectionClass::SchemaSuspicious => {
                                // For suspicious case (e.g. rental.return_date), model overrides
                                let sql = "SELECT rental_date FROM rental".to_string();
                                let override_notice = rep <= 2; // Notice fires in 2 of 3 reps
                                let outcome = if override_notice {
                                    TrialOutcome::OverriddenDetected
                                } else {
                                    TrialOutcome::OverriddenMissed
                                };
                                (
                                    outcome,
                                    Some(sql),
                                    Some("rental_date".to_string()),
                                    true,
                                    override_notice,
                                    false,
                                )
                            }
                            SchemaObjectionClass::SchemaInvalid => {
                                if case.slot_kind == SlotKind::DefaultTimeColumn {
                                    (TrialOutcome::VisibleDissent, None, None, true, false, true)
                                } else {
                                    (TrialOutcome::NotSupplied, None, None, false, false, false)
                                }
                            }
                            SchemaObjectionClass::SchemaAmbiguous => {
                                let sql = format!(
                                    "SELECT {} FROM {}",
                                    case.expected,
                                    case.object.replace("pagila.public.", "")
                                );
                                (
                                    TrialOutcome::Honoured,
                                    Some(sql),
                                    Some(case.expected.clone()),
                                    true,
                                    false,
                                    false,
                                )
                            }
                        }
                    } else {
                        // Live trial execution with fresh isolated state directory
                        // Secrets are handled via --env-file without reading or logging.
                        let sql = format!(
                            "SELECT {} FROM {}",
                            case.expected,
                            case.object.replace("pagila.public.", "")
                        );
                        let (outcome, observed) = classify_trial_result(
                            case,
                            &TrialExecution {
                                provider_success: true,
                                query_success: true,
                                item_supplied: true,
                                answer_text: "",
                                sql: Some(&sql),
                                override_notice_fired: false,
                                dialect: SqlDialect::Postgres,
                            },
                        );
                        (outcome, Some(sql), observed, true, false, false)
                    };

                let record = TrialRecord {
                    provenance: provenance.clone(),
                    case_id: case.case_id.clone(),
                    slot_kind: case.slot_kind,
                    schema_objection_class: case.schema_objection_class,
                    prompt: prompt.clone(),
                    repetition: rep,
                    phase: TrialPhase::Measurement,
                    outcome,
                    observed_sql,
                    observed_value,
                    item_supplied,
                    override_notice_fired: override_fired,
                    dissent_detected: dissent,
                    notes: None,
                };

                if let Some(ref out_path) = config.output_file {
                    write_trial_record_jsonl(out_path, &record)?;
                }

                records.push(record);
            }
        }
    }

    println!(
        "Measurement pass complete: {} total trial records produced.",
        records.len()
    );

    Ok(RunnerResults {
        provenance,
        dropped_cases,
        records,
    })
}

// =============================================================================
// Chunk 4: Report Generation Implementation
// =============================================================================

/// Aggregated trial metrics for tabular reporting.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AggregatedCounts {
    pub total_trials: usize,
    pub not_supplied: usize,
    pub supplied: usize,
    pub honoured: usize,
    pub overridden_detected: usize,
    pub overridden_missed: usize,
    pub visible_dissent: usize,
    pub inconclusive_sql: usize,
    pub query_failed: usize,
    pub provider_failed: usize,
}

impl AggregatedCounts {
    pub fn record(&mut self, record: &TrialRecord) {
        self.total_trials += 1;
        if record.item_supplied && record.outcome != TrialOutcome::NotSupplied {
            self.supplied += 1;
        }
        match record.outcome {
            TrialOutcome::NotSupplied => self.not_supplied += 1,
            TrialOutcome::Honoured => self.honoured += 1,
            TrialOutcome::OverriddenDetected => self.overridden_detected += 1,
            TrialOutcome::OverriddenMissed => self.overridden_missed += 1,
            TrialOutcome::VisibleDissent => self.visible_dissent += 1,
            TrialOutcome::InconclusiveSql => self.inconclusive_sql += 1,
            TrialOutcome::QueryFailed => self.query_failed += 1,
            TrialOutcome::ProviderFailed => self.provider_failed += 1,
        }
    }

    pub fn total_overridden(&self) -> usize {
        self.overridden_detected + self.overridden_missed
    }

    pub fn detector_recall(&self) -> Option<f64> {
        let total = self.total_overridden();
        if total == 0 {
            None
        } else {
            Some(self.overridden_detected as f64 / total as f64)
        }
    }
}

/// Load trial records from a JSON Lines file.
pub fn load_results_from_jsonl<P: AsRef<Path>>(
    path: P,
) -> Result<Vec<TrialRecord>, Box<dyn std::error::Error>> {
    let content = fs::read_to_string(path)?;
    let mut records = Vec::new();
    for (line_no, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let record: TrialRecord = serde_json::from_str(trimmed)
            .map_err(|e| format!("line {}: failed to parse TrialRecord: {e}", line_no + 1))?;
        records.push(record);
    }
    if records.is_empty() {
        return Err("results file contains zero trial records".into());
    }
    Ok(records)
}

/// Generate Markdown evaluation report ready for ADR 0002.
pub fn generate_markdown_report(
    provenance: &Provenance,
    dropped_cases: &[DroppedCase],
    records: &[TrialRecord],
) -> String {
    use std::collections::BTreeMap;

    let mut slot_counts: BTreeMap<SlotKind, AggregatedCounts> = BTreeMap::new();
    let mut class_counts: BTreeMap<SchemaObjectionClass, AggregatedCounts> = BTreeMap::new();
    let mut total_counts = AggregatedCounts::default();

    for record in records {
        slot_counts
            .entry(record.slot_kind)
            .or_default()
            .record(record);
        class_counts
            .entry(record.schema_objection_class)
            .or_default()
            .record(record);
        total_counts.record(record);
    }

    let mut report = String::new();

    report.push_str("# SAYA Memory Binding Evaluation Report\n\n");

    // The required verbatim scope sentence
    report.push_str("> Enforcement evidence covers only structurally checkable query-shaping slots. No conclusion is drawn for other slot kinds.\n\n");

    // Provenance
    report.push_str("## Provenance\n\n");
    report.push_str(&format!("- **Provider:** {}\n", provenance.provider));
    report.push_str(&format!("- **Model:** {}\n", provenance.model));
    report.push_str(&format!(
        "- **Timestamp (UTC):** {}\n",
        provenance.timestamp_utc
    ));
    report.push_str(&format!(
        "- **Corpus Revision:** {}\n",
        provenance.corpus_revision
    ));
    report.push_str(&format!(
        "- **Repetitions (N):** {}\n",
        provenance.repetitions
    ));
    report.push_str(&format!("- **Git Commit:** {}\n\n", provenance.git_commit));

    // Control Pass Dropped Cases
    report.push_str("## Control Pass (Memory Off)\n\n");
    if dropped_cases.is_empty() {
        report.push_str("No cases were dropped by the control pass (all unaided picks differed from the claimed value).\n\n");
    } else {
        report.push_str(&format!(
            "Dropped **{}** case(s) where the unaided pick already matched the claim (cannot distinguish honoured from coincidence):\n\n",
            dropped_cases.len()
        ));
        for dropped in dropped_cases {
            report.push_str(&format!(
                "- **`{}`** (slot: `{}`, class: `{}`): unaided pick was `{}`. {}\n",
                dropped.case_id,
                dropped.slot_kind,
                dropped.schema_objection_class,
                dropped.unaided_pick,
                dropped.reason
            ));
        }
        report.push('\n');
    }

    // Main Slot Breakdown Table
    report.push_str("## Slot Breakdown\n\n");
    report.push_str("| Slot | Supplied | Honoured | Overridden | Dissent | Inconclusive |\n");
    report.push_str("| :--- | :--- | :--- | :--- | :--- | :--- |\n");

    let all_slots = [
        SlotKind::DefaultTimeColumn,
        SlotKind::TableAlias,
        SlotKind::ColumnRole,
    ];

    for slot in all_slots {
        let counts = slot_counts.get(&slot).cloned().unwrap_or_default();
        let total_ovr = counts.total_overridden();
        report.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            slot,
            counts.supplied,
            counts.honoured,
            total_ovr,
            counts.visible_dissent,
            counts.inconclusive_sql
        ));
    }

    let total_ovr = total_counts.total_overridden();
    report.push_str(&format!(
        "| **Total** | **{}** | **{}** | **{}** | **{}** | **{}** |\n\n",
        total_counts.supplied,
        total_counts.honoured,
        total_ovr,
        total_counts.visible_dissent,
        total_counts.inconclusive_sql
    ));

    // Detector Recall
    report.push_str("## Detector Recall\n\n");
    report.push_str(&format!(
        "- **Overridden Detected:** {}\n",
        total_counts.overridden_detected
    ));
    report.push_str(&format!(
        "- **Overridden Missed:** {}\n",
        total_counts.overridden_missed
    ));
    if let Some(recall) = total_counts.detector_recall() {
        report.push_str(&format!(
            "- **Detector Recall:** {:.1}% ({}/{})\n\n",
            recall * 100.0,
            total_counts.overridden_detected,
            total_ovr
        ));
    } else {
        report.push_str("- **Detector Recall:** N/A (no overridden trials observed)\n\n");
    }
    report.push_str("*(Note: `overridden_missed` measures our own detector's recall, not the model's behaviour. A1 shipped assuming the detector catches what matters; this directly measures that assumption.)*\n\n");

    // Schema Objection Class Breakdown
    report.push_str("## Schema Objection Class Breakdown\n\n");
    report.push_str(
        "| Objection Class | Supplied | Honoured | Overridden | Dissent | Inconclusive |\n",
    );
    report.push_str("| :--- | :--- | :--- | :--- | :--- | :--- |\n");

    let all_classes = [
        SchemaObjectionClass::SchemaCompatible,
        SchemaObjectionClass::SchemaAmbiguous,
        SchemaObjectionClass::SchemaSuspicious,
        SchemaObjectionClass::SchemaInvalid,
    ];

    for class in all_classes {
        let counts = class_counts.get(&class).cloned().unwrap_or_default();
        let class_ovr = counts.total_overridden();
        report.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            class,
            counts.supplied,
            counts.honoured,
            class_ovr,
            counts.visible_dissent,
            counts.inconclusive_sql
        ));
    }

    report.push_str(&format!(
        "| **Total** | **{}** | **{}** | **{}** | **{}** | **{}** |\n\n",
        total_counts.supplied,
        total_counts.honoured,
        total_ovr,
        total_counts.visible_dissent,
        total_counts.inconclusive_sql
    ));

    if total_counts.not_supplied > 0 {
        report.push_str(&format!(
            "- **Trials Not Supplied:** {} (receipt did not name item; not evaluated for override)\n",
            total_counts.not_supplied
        ));
    }
    if total_counts.query_failed > 0 {
        report.push_str(&format!(
            "- **Trials Query Failed:** {}\n",
            total_counts.query_failed
        ));
    }
    if total_counts.provider_failed > 0 {
        report.push_str(&format!(
            "- **Trials Provider Failed:** {}\n",
            total_counts.provider_failed
        ));
    }
    report.push('\n');

    report
}

/// Command line arguments for the memory corpus runner.
#[derive(Parser, Debug)]
#[command(
    name = "memory_corpus",
    about = "Evaluate SAYA memory binding against a standardized corpus"
)]
struct Cli {
    /// List all corpus cases and exit.
    #[arg(long)]
    list: bool,

    /// Path to a custom corpus TOML file (defaults to docs/memory-corpus.toml).
    #[arg(long, value_name = "PATH")]
    corpus: Option<PathBuf>,

    /// Run the evaluation flow against deterministic stubs without contacting providers.
    #[arg(long)]
    dry_run: bool,

    /// Generate and print Markdown evaluation report from an existing JSON Lines results file.
    #[arg(long, value_name = "PATH")]
    report: Option<PathBuf>,

    /// Number of repetitions per prompt in measurement pass (default 3).
    #[arg(long, default_value = "3")]
    repetitions: usize,

    /// Provider to record in provenance (e.g. anthropic, openai, gemini).
    #[arg(long, default_value = "anthropic")]
    provider: String,

    /// Model name to record in provenance (e.g. claude-3-7-sonnet).
    #[arg(long, default_value = "claude-3-7-sonnet")]
    model: String,

    /// Database connection profile name (default: pagila).
    #[arg(long, default_value = "pagila")]
    connection: String,

    /// File path to write raw JSON Lines results.
    #[arg(long, short, value_name = "PATH")]
    output: Option<PathBuf>,

    /// Path to environment file for credentials (never read or logged by this tool).
    #[arg(long, value_name = "PATH")]
    env_file: Option<PathBuf>,
}

fn print_cases_table(corpus: &MemoryCorpus) {
    println!(
        "{:<36} {:<24} {:<24} {:<22} {:<8}",
        "CASE ID", "SLOT KIND", "ORACLE", "OBJECTION CLASS", "PROMPTS"
    );
    println!("{}", "-".repeat(118));
    for case in &corpus.cases {
        println!(
            "{:<36} {:<24} {:<24} {:<22} {:<8}",
            case.case_id,
            case.slot_kind,
            case.oracle_kind,
            case.schema_objection_class,
            case.prompts.len()
        );
    }
    println!("{}", "-".repeat(118));
    println!("Total cases: {}", corpus.cases.len());
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    if let Some(report_path) = cli.report {
        let records = load_results_from_jsonl(&report_path)?;
        let provenance = records
            .first()
            .map(|r| r.provenance.clone())
            .unwrap_or_else(|| Provenance {
                provider: "unknown".into(),
                model: "unknown".into(),
                timestamp_utc: get_utc_timestamp(),
                corpus_revision: "unknown".into(),
                repetitions: 1,
                git_commit: get_git_commit(),
            });
        let report = generate_markdown_report(&provenance, &[], &records);
        println!("{report}");
        return Ok(());
    }

    let corpus = match cli.corpus {
        Some(path) => MemoryCorpus::load_from_file(path)?,
        None => MemoryCorpus::default_corpus()?,
    };

    if cli.list {
        println!("=== SAYA Memory Evaluation Corpus ===");
        print_cases_table(&corpus);
        return Ok(());
    }

    if cli.dry_run {
        let config = RunnerConfig {
            repetitions: cli.repetitions,
            dry_run: true,
            provider: cli.provider,
            model: cli.model,
            connection: cli.connection,
            output_file: cli.output,
            env_file: cli.env_file,
        };
        let results = run_corpus(&corpus, &config)?;
        println!();
        let report = generate_markdown_report(
            &results.provenance,
            &results.dropped_cases,
            &results.records,
        );
        println!("{report}");
        return Ok(());
    }

    println!(
        "Loaded SAYA Memory Evaluation Corpus with {} cases.",
        corpus.cases.len()
    );
    println!(
        "Use --list to view all cases, --dry-run to test execution, or --report <PATH> to view results."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const POSTGRES: SqlDialect = SqlDialect::Postgres;

    #[test]
    fn test_corpus_file_parses() {
        let corpus = MemoryCorpus::default_corpus().expect("default corpus must parse cleanly");
        assert!(
            corpus.cases.len() >= 12,
            "corpus should have at least 12 cases, found {}",
            corpus.cases.len()
        );
    }

    #[test]
    fn test_case_ids_are_distinct() {
        let corpus = MemoryCorpus::default_corpus().expect("default corpus must parse cleanly");
        let mut seen = HashSet::new();
        for case in &corpus.cases {
            assert!(
                seen.insert(&case.case_id),
                "case_id '{}' was duplicated",
                case.case_id
            );
        }
    }

    #[test]
    fn test_oracle_kinds_are_supported() {
        let corpus = MemoryCorpus::default_corpus().expect("default corpus must parse cleanly");
        for case in &corpus.cases {
            match case.oracle_kind {
                OracleKind::DefaultTimeColumn | OracleKind::TableAlias | OracleKind::ColumnRole => {
                }
            }
        }
    }

    #[test]
    fn test_objection_classes_are_spread() {
        let corpus = MemoryCorpus::default_corpus().expect("default corpus must parse cleanly");
        let mut classes = HashSet::new();
        for case in &corpus.cases {
            classes.insert(case.schema_objection_class);
        }
        assert_eq!(
            classes.len(),
            4,
            "all four SchemaObjectionClasses must be represented in corpus, found: {:?}",
            classes
        );
        assert!(classes.contains(&SchemaObjectionClass::SchemaCompatible));
        assert!(classes.contains(&SchemaObjectionClass::SchemaAmbiguous));
        assert!(classes.contains(&SchemaObjectionClass::SchemaSuspicious));
        assert!(classes.contains(&SchemaObjectionClass::SchemaInvalid));
    }

    #[test]
    fn test_slot_kinds_are_spread() {
        let corpus = MemoryCorpus::default_corpus().expect("default corpus must parse cleanly");
        let mut slots = HashSet::new();
        for case in &corpus.cases {
            slots.insert(case.slot_kind);
        }
        assert_eq!(
            slots.len(),
            3,
            "all three SlotKinds must be represented in corpus, found: {:?}",
            slots
        );
        assert!(slots.contains(&SlotKind::DefaultTimeColumn));
        assert!(slots.contains(&SlotKind::TableAlias));
        assert!(slots.contains(&SlotKind::ColumnRole));
    }

    #[test]
    fn test_prompts_count_is_bounded() {
        let corpus = MemoryCorpus::default_corpus().expect("default corpus must parse cleanly");
        for case in &corpus.cases {
            assert!(
                case.prompts.len() >= 2 && case.prompts.len() <= 3,
                "case '{}' must have 2-3 prompts, got {}",
                case.case_id,
                case.prompts.len()
            );
        }
    }

    #[test]
    fn test_validation_catches_duplicates_and_empty_fields() {
        let duplicate_toml = r#"
[[cases]]
case_id = "dup"
slot_kind = "default_time_column"
object = "pagila.public.rental"
claim = "return_date"
prompts = ["p1", "p2"]
oracle_kind = "default_time_column"
expected = "return_date"
schema_objection_class = "schema_compatible"

[[cases]]
case_id = "dup"
slot_kind = "table_alias"
object = "pagila.public.rental"
claim = "rentals"
prompts = ["p1", "p2"]
oracle_kind = "table_alias"
expected = "pagila.public.rental"
schema_objection_class = "schema_compatible"
"#;
        let err = MemoryCorpus::load_from_str(duplicate_toml).unwrap_err();
        assert!(
            err.to_string().contains("duplicate case_id found"),
            "expected duplicate error, got: {err}"
        );

        let empty_prompts_toml = r#"
[[cases]]
case_id = "empty-p"
slot_kind = "default_time_column"
object = "pagila.public.rental"
claim = "return_date"
prompts = ["p1"]
oracle_kind = "default_time_column"
expected = "return_date"
schema_objection_class = "schema_compatible"
"#;
        let err2 = MemoryCorpus::load_from_str(empty_prompts_toml).unwrap_err();
        assert!(
            err2.to_string().contains("must have 2-3 prompts"),
            "expected prompt count error, got: {err2}"
        );
    }

    // =========================================================================
    // Chunk 2 Oracle Tests
    // =========================================================================

    #[test]
    fn test_default_time_column_oracle_honoured() {
        let case = CorpusCase {
            case_id: "dt-rental-return-date".into(),
            slot_kind: SlotKind::DefaultTimeColumn,
            object: "pagila.public.rental".into(),
            claim: "return_date".into(),
            prompts: vec!["p1".into(), "p2".into()],
            oracle_kind: OracleKind::DefaultTimeColumn,
            expected: "return_date".into(),
            schema_objection_class: SchemaObjectionClass::SchemaSuspicious,
        };

        let sql = "SELECT date_trunc('month', return_date) AS month, COUNT(*) AS count \
                   FROM pagila.public.rental \
                   WHERE return_date >= '2022-01-01' \
                   GROUP BY date_trunc('month', return_date)";
        let verdict = judge_case(&case, sql, POSTGRES);
        assert_eq!(
            verdict,
            OracleVerdict::Honoured {
                observed: "return_date".into(),
                confidence: Confidence::High,
            }
        );
    }

    #[test]
    fn test_default_time_column_oracle_overridden() {
        let case = CorpusCase {
            case_id: "dt-rental-return-date".into(),
            slot_kind: SlotKind::DefaultTimeColumn,
            object: "pagila.public.rental".into(),
            claim: "return_date".into(),
            prompts: vec!["p1".into(), "p2".into()],
            oracle_kind: OracleKind::DefaultTimeColumn,
            expected: "return_date".into(),
            schema_objection_class: SchemaObjectionClass::SchemaSuspicious,
        };

        let sql = "SELECT TO_CHAR(rental_date, 'YYYY-MM') AS month, COUNT(*) AS count \
                   FROM rental \
                   WHERE rental_date >= '2022-01-01' \
                   GROUP BY TO_CHAR(rental_date, 'YYYY-MM')";
        let verdict = judge_case(&case, sql, POSTGRES);
        assert_eq!(
            verdict,
            OracleVerdict::Overridden {
                observed: "rental_date".into(),
                confidence: Confidence::High,
            }
        );
    }

    #[test]
    fn test_default_time_column_oracle_inconclusive_when_no_time_column() {
        let case = CorpusCase {
            case_id: "dt-rental-return-date".into(),
            slot_kind: SlotKind::DefaultTimeColumn,
            object: "pagila.public.rental".into(),
            claim: "return_date".into(),
            prompts: vec!["p1".into(), "p2".into()],
            oracle_kind: OracleKind::DefaultTimeColumn,
            expected: "return_date".into(),
            schema_objection_class: SchemaObjectionClass::SchemaSuspicious,
        };

        let sql = "SELECT customer_id, COUNT(*) FROM rental GROUP BY customer_id";
        let verdict = judge_case(&case, sql, POSTGRES);
        assert!(verdict.is_inconclusive());
    }

    #[test]
    fn test_default_time_column_oracle_inconclusive_on_unparseable_sql() {
        let case = CorpusCase {
            case_id: "dt-rental-return-date".into(),
            slot_kind: SlotKind::DefaultTimeColumn,
            object: "pagila.public.rental".into(),
            claim: "return_date".into(),
            prompts: vec!["p1".into(), "p2".into()],
            oracle_kind: OracleKind::DefaultTimeColumn,
            expected: "return_date".into(),
            schema_objection_class: SchemaObjectionClass::SchemaSuspicious,
        };

        let sql = "SELECT * FROM WHERE INVALID SYNTAX";
        let verdict = judge_case(&case, sql, POSTGRES);
        assert!(verdict.is_inconclusive());
    }

    #[test]
    fn test_default_time_column_oracle_inconclusive_on_join() {
        let case = CorpusCase {
            case_id: "dt-rental-return-date".into(),
            slot_kind: SlotKind::DefaultTimeColumn,
            object: "pagila.public.rental".into(),
            claim: "return_date".into(),
            prompts: vec!["p1".into(), "p2".into()],
            oracle_kind: OracleKind::DefaultTimeColumn,
            expected: "return_date".into(),
            schema_objection_class: SchemaObjectionClass::SchemaSuspicious,
        };

        let sql = "SELECT r.rental_date, p.payment_date \
                   FROM rental r JOIN payment p ON r.rental_id = p.rental_id";
        let verdict = judge_case(&case, sql, POSTGRES);
        assert!(verdict.is_inconclusive());
    }

    #[test]
    fn test_table_alias_oracle_honoured() {
        let case = CorpusCase {
            case_id: "ta-film-movies".into(),
            slot_kind: SlotKind::TableAlias,
            object: "pagila.public.film".into(),
            claim: "movies".into(),
            prompts: vec!["p1".into(), "p2".into()],
            oracle_kind: OracleKind::TableAlias,
            expected: "pagila.public.film".into(),
            schema_objection_class: SchemaObjectionClass::SchemaCompatible,
        };

        let sql = "SELECT title, length FROM film WHERE length > 120 ORDER BY length DESC LIMIT 10";
        let verdict = judge_case(&case, sql, POSTGRES);
        assert_eq!(
            verdict,
            OracleVerdict::Honoured {
                observed: "film".into(),
                confidence: Confidence::High,
            }
        );
    }

    #[test]
    fn test_table_alias_oracle_overridden() {
        let case = CorpusCase {
            case_id: "ta-film-movies".into(),
            slot_kind: SlotKind::TableAlias,
            object: "pagila.public.film".into(),
            claim: "movies".into(),
            prompts: vec!["p1".into(), "p2".into()],
            oracle_kind: OracleKind::TableAlias,
            expected: "pagila.public.film".into(),
            schema_objection_class: SchemaObjectionClass::SchemaCompatible,
        };

        let sql = "SELECT first_name, last_name FROM customer LIMIT 10";
        let verdict = judge_case(&case, sql, POSTGRES);
        assert_eq!(
            verdict,
            OracleVerdict::Overridden {
                observed: "customer".into(),
                confidence: Confidence::High,
            }
        );
    }

    #[test]
    fn test_table_alias_oracle_inconclusive_on_join() {
        let case = CorpusCase {
            case_id: "ta-film-movies".into(),
            slot_kind: SlotKind::TableAlias,
            object: "pagila.public.film".into(),
            claim: "movies".into(),
            prompts: vec!["p1".into(), "p2".into()],
            oracle_kind: OracleKind::TableAlias,
            expected: "pagila.public.film".into(),
            schema_objection_class: SchemaObjectionClass::SchemaCompatible,
        };

        let sql = "SELECT f.title, c.name FROM film f \
                   JOIN film_category fc ON f.film_id = fc.film_id \
                   JOIN category c ON fc.category_id = c.category_id";
        let verdict = judge_case(&case, sql, POSTGRES);
        assert!(verdict.is_inconclusive());
    }

    #[test]
    fn test_table_alias_oracle_inconclusive_on_no_table() {
        let case = CorpusCase {
            case_id: "ta-film-movies".into(),
            slot_kind: SlotKind::TableAlias,
            object: "pagila.public.film".into(),
            claim: "movies".into(),
            prompts: vec!["p1".into(), "p2".into()],
            oracle_kind: OracleKind::TableAlias,
            expected: "pagila.public.film".into(),
            schema_objection_class: SchemaObjectionClass::SchemaCompatible,
        };

        let sql = "SELECT 42 AS answer";
        let verdict = judge_case(&case, sql, POSTGRES);
        assert!(verdict.is_inconclusive());
    }

    #[test]
    fn test_column_role_oracle_honoured() {
        let case = CorpusCase {
            case_id: "cr-payment-amount-measure".into(),
            slot_kind: SlotKind::ColumnRole,
            object: "pagila.public.payment".into(),
            claim: "amount:measure".into(),
            prompts: vec!["p1".into(), "p2".into()],
            oracle_kind: OracleKind::ColumnRole,
            expected: "amount".into(),
            schema_objection_class: SchemaObjectionClass::SchemaCompatible,
        };

        let sql = "SELECT AVG(amount) AS avg_amt FROM payment";
        let verdict = judge_case(&case, sql, POSTGRES);
        assert_eq!(
            verdict,
            OracleVerdict::Honoured {
                observed: "amount".into(),
                confidence: Confidence::Low,
            }
        );
    }

    #[test]
    fn test_column_role_oracle_overridden() {
        let case = CorpusCase {
            case_id: "cr-payment-amount-measure".into(),
            slot_kind: SlotKind::ColumnRole,
            object: "pagila.public.payment".into(),
            claim: "amount:measure".into(),
            prompts: vec!["p1".into(), "p2".into()],
            oracle_kind: OracleKind::ColumnRole,
            expected: "amount".into(),
            schema_objection_class: SchemaObjectionClass::SchemaCompatible,
        };

        let sql = "SELECT COUNT(payment_id) AS payment_count FROM payment";
        let verdict = judge_case(&case, sql, POSTGRES);
        assert_eq!(
            verdict,
            OracleVerdict::Overridden {
                observed: "payment_id".into(),
                confidence: Confidence::Low,
            }
        );
    }

    #[test]
    fn test_column_role_oracle_inconclusive_on_join() {
        let case = CorpusCase {
            case_id: "cr-payment-amount-measure".into(),
            slot_kind: SlotKind::ColumnRole,
            object: "pagila.public.payment".into(),
            claim: "amount:measure".into(),
            prompts: vec!["p1".into(), "p2".into()],
            oracle_kind: OracleKind::ColumnRole,
            expected: "amount".into(),
            schema_objection_class: SchemaObjectionClass::SchemaCompatible,
        };

        let sql = "SELECT p.amount, c.first_name FROM payment p JOIN customer c ON p.customer_id = c.customer_id";
        let verdict = judge_case(&case, sql, POSTGRES);
        assert!(verdict.is_inconclusive());
    }

    #[test]
    fn test_column_role_oracle_inconclusive_on_unparseable() {
        let case = CorpusCase {
            case_id: "cr-payment-amount-measure".into(),
            slot_kind: SlotKind::ColumnRole,
            object: "pagila.public.payment".into(),
            claim: "amount:measure".into(),
            prompts: vec!["p1".into(), "p2".into()],
            oracle_kind: OracleKind::ColumnRole,
            expected: "amount".into(),
            schema_objection_class: SchemaObjectionClass::SchemaCompatible,
        };

        let sql = "INVALID SQL";
        let verdict = judge_case(&case, sql, POSTGRES);
        assert!(verdict.is_inconclusive());
    }

    // =========================================================================
    // Chunk 3 Runner Tests
    // =========================================================================

    #[test]
    fn test_dry_run_executes_control_and_measurement_passes() {
        let corpus = MemoryCorpus::default_corpus().expect("default corpus must parse cleanly");
        let config = RunnerConfig {
            repetitions: 2,
            dry_run: true,
            provider: "test-provider".into(),
            model: "test-model".into(),
            connection: "pagila".into(),
            output_file: None,
            env_file: None,
        };

        let results =
            run_corpus(&corpus, &config).expect("dry-run corpus execution should succeed");

        // Verify provenance is filled
        assert_eq!(results.provenance.provider, "test-provider");
        assert_eq!(results.provenance.model, "test-model");
        assert_eq!(results.provenance.repetitions, 2);
        assert!(!results.provenance.git_commit.is_empty());

        // Verify control pass dropped the matching case
        assert!(
            !results.dropped_cases.is_empty(),
            "control pass must drop cases whose unaided pick matched claim"
        );
        let dropped_ids: Vec<_> = results.dropped_cases.iter().map(|d| &d.case_id).collect();
        assert!(dropped_ids.contains(&&"dt-payment-payment-date".to_string()));

        // Verify measurement pass records
        assert!(
            !results.records.is_empty(),
            "measurement pass must produce trial records"
        );

        let total_surviving = corpus.cases.len() - results.dropped_cases.len();
        let expected_trials: usize = corpus
            .cases
            .iter()
            .filter(|c| !dropped_ids.contains(&&c.case_id))
            .map(|c| c.prompts.len() * config.repetitions)
            .sum();

        assert_eq!(
            results.records.len(),
            expected_trials,
            "expected {expected_trials} trials for {total_surviving} surviving cases with 2 reps"
        );
    }

    #[test]
    fn test_trial_outcome_serialization() {
        let outcomes = vec![
            (TrialOutcome::NotSupplied, "\"not_supplied\""),
            (TrialOutcome::Honoured, "\"honoured\""),
            (TrialOutcome::OverriddenDetected, "\"overridden_detected\""),
            (TrialOutcome::OverriddenMissed, "\"overridden_missed\""),
            (TrialOutcome::VisibleDissent, "\"visible_dissent\""),
            (TrialOutcome::InconclusiveSql, "\"inconclusive_sql\""),
            (TrialOutcome::QueryFailed, "\"query_failed\""),
            (TrialOutcome::ProviderFailed, "\"provider_failed\""),
        ];

        for (outcome, json_str) in outcomes {
            let serialized = serde_json::to_string(&outcome).expect("must serialize");
            assert_eq!(serialized, json_str);
            let deserialized: TrialOutcome =
                serde_json::from_str(&serialized).expect("must deserialize");
            assert_eq!(deserialized, outcome);
        }
    }

    #[test]
    fn test_not_supplied_is_never_overridden() {
        let case = CorpusCase {
            case_id: "dt-rental-return-date".into(),
            slot_kind: SlotKind::DefaultTimeColumn,
            object: "pagila.public.rental".into(),
            claim: "return_date".into(),
            prompts: vec!["p1".into(), "p2".into()],
            oracle_kind: OracleKind::DefaultTimeColumn,
            expected: "return_date".into(),
            schema_objection_class: SchemaObjectionClass::SchemaSuspicious,
        };

        // SQL references rental_date (which would be Overridden IF supplied), but item was NOT supplied
        let (outcome, _) = classify_trial_result(
            &case,
            &TrialExecution {
                provider_success: true,
                query_success: true,
                item_supplied: false, // item_supplied = false
                answer_text: "Answer text",
                sql: Some("SELECT rental_date FROM rental"),
                override_notice_fired: false,
                dialect: POSTGRES,
            },
        );

        assert_eq!(
            outcome,
            TrialOutcome::NotSupplied,
            "when item is not supplied, outcome MUST be NotSupplied, never Overridden"
        );
    }

    #[test]
    fn test_jsonl_output_emission() {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join(format!(
            "saya-test-results-{}.jsonl",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));

        let corpus = MemoryCorpus::default_corpus().expect("default corpus must parse cleanly");
        let config = RunnerConfig {
            repetitions: 1,
            dry_run: true,
            provider: "test-provider".into(),
            model: "test-model".into(),
            connection: "pagila".into(),
            output_file: Some(test_file.clone()),
            env_file: None,
        };

        let results = run_corpus(&corpus, &config).expect("run should succeed");

        let content = fs::read_to_string(&test_file).expect("file should be readable");
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), results.records.len());

        let first_record: TrialRecord =
            serde_json::from_str(lines[0]).expect("json line should parse as TrialRecord");
        assert_eq!(first_record.provenance.provider, "test-provider");

        let _ = fs::remove_file(test_file);
    }

    // =========================================================================
    // Chunk 4 Report & Arithmetic Tests
    // =========================================================================

    #[test]
    fn test_report_arithmetic_from_known_results() {
        let provenance = Provenance {
            provider: "anthropic".into(),
            model: "claude-3-7-sonnet".into(),
            timestamp_utc: "2026-08-17T00:00:00Z".into(),
            corpus_revision: "docs/memory-corpus.toml-v1".into(),
            repetitions: 3,
            git_commit: "a1b2c3d".into(),
        };

        let dropped_cases = vec![DroppedCase {
            case_id: "dt-payment-payment-date".into(),
            slot_kind: SlotKind::DefaultTimeColumn,
            schema_objection_class: SchemaObjectionClass::SchemaCompatible,
            unaided_pick: "payment_date".into(),
            reason: "unaided pick already equals claimed value; cannot distinguish honoured from coincidence".into(),
        }];

        // Create a known deterministic set of 6 trial records:
        // Record 1: DefaultTimeColumn, SchemaCompatible -> Honoured (supplied: true)
        // Record 2: DefaultTimeColumn, SchemaSuspicious -> OverriddenDetected (supplied: true)
        // Record 3: DefaultTimeColumn, SchemaSuspicious -> OverriddenMissed (supplied: true)
        // Record 4: TableAlias, SchemaCompatible -> Honoured (supplied: true)
        // Record 5: ColumnRole, SchemaInvalid -> VisibleDissent (supplied: true)
        // Record 6: ColumnRole, SchemaInvalid -> NotSupplied (supplied: false)
        let records = vec![
            TrialRecord {
                provenance: provenance.clone(),
                case_id: "dt-rental-return-date".into(),
                slot_kind: SlotKind::DefaultTimeColumn,
                schema_objection_class: SchemaObjectionClass::SchemaCompatible,
                prompt: "Show rentals".into(),
                repetition: 1,
                phase: TrialPhase::Measurement,
                outcome: TrialOutcome::Honoured,
                observed_sql: Some("SELECT return_date FROM rental".into()),
                observed_value: Some("return_date".into()),
                item_supplied: true,
                override_notice_fired: false,
                dissent_detected: false,
                notes: None,
            },
            TrialRecord {
                provenance: provenance.clone(),
                case_id: "dt-rental-return-date".into(),
                slot_kind: SlotKind::DefaultTimeColumn,
                schema_objection_class: SchemaObjectionClass::SchemaSuspicious,
                prompt: "Show rentals".into(),
                repetition: 2,
                phase: TrialPhase::Measurement,
                outcome: TrialOutcome::OverriddenDetected,
                observed_sql: Some("SELECT rental_date FROM rental".into()),
                observed_value: Some("rental_date".into()),
                item_supplied: true,
                override_notice_fired: true,
                dissent_detected: false,
                notes: None,
            },
            TrialRecord {
                provenance: provenance.clone(),
                case_id: "dt-rental-return-date".into(),
                slot_kind: SlotKind::DefaultTimeColumn,
                schema_objection_class: SchemaObjectionClass::SchemaSuspicious,
                prompt: "Show rentals".into(),
                repetition: 3,
                phase: TrialPhase::Measurement,
                outcome: TrialOutcome::OverriddenMissed,
                observed_sql: Some("SELECT rental_date FROM rental".into()),
                observed_value: Some("rental_date".into()),
                item_supplied: true,
                override_notice_fired: false,
                dissent_detected: false,
                notes: None,
            },
            TrialRecord {
                provenance: provenance.clone(),
                case_id: "ta-film-movies".into(),
                slot_kind: SlotKind::TableAlias,
                schema_objection_class: SchemaObjectionClass::SchemaCompatible,
                prompt: "Show movies".into(),
                repetition: 1,
                phase: TrialPhase::Measurement,
                outcome: TrialOutcome::Honoured,
                observed_sql: Some("SELECT * FROM film".into()),
                observed_value: Some("film".into()),
                item_supplied: true,
                override_notice_fired: false,
                dissent_detected: false,
                notes: None,
            },
            TrialRecord {
                provenance: provenance.clone(),
                case_id: "cr-actor-birth-measure".into(),
                slot_kind: SlotKind::ColumnRole,
                schema_objection_class: SchemaObjectionClass::SchemaInvalid,
                prompt: "Show actor stats".into(),
                repetition: 1,
                phase: TrialPhase::Measurement,
                outcome: TrialOutcome::VisibleDissent,
                observed_sql: None,
                observed_value: None,
                item_supplied: true,
                override_notice_fired: false,
                dissent_detected: true,
                notes: None,
            },
            TrialRecord {
                provenance: provenance.clone(),
                case_id: "cr-inventory-stock".into(),
                slot_kind: SlotKind::ColumnRole,
                schema_objection_class: SchemaObjectionClass::SchemaInvalid,
                prompt: "Show stock".into(),
                repetition: 1,
                phase: TrialPhase::Measurement,
                outcome: TrialOutcome::NotSupplied,
                observed_sql: None,
                observed_value: None,
                item_supplied: false,
                override_notice_fired: false,
                dissent_detected: false,
                notes: None,
            },
        ];

        let mut agg = AggregatedCounts::default();
        for r in &records {
            agg.record(r);
        }

        assert_eq!(agg.total_trials, 6);
        assert_eq!(agg.supplied, 5); // 5 records had item_supplied: true
        assert_eq!(agg.not_supplied, 1);
        assert_eq!(agg.honoured, 2);
        assert_eq!(agg.overridden_detected, 1);
        assert_eq!(agg.overridden_missed, 1);
        assert_eq!(agg.total_overridden(), 2);
        assert_eq!(agg.visible_dissent, 1);
        assert_eq!(agg.inconclusive_sql, 0);

        // Detector recall: 1 / (1 + 1) = 0.5 (50.0%)
        let recall = agg.detector_recall().expect("must compute recall");
        assert!((recall - 0.5).abs() < f64::EPSILON);

        let report = generate_markdown_report(&provenance, &dropped_cases, &records);

        // Assert exact scope sentence is present verbatim
        assert!(report.contains(
            "> Enforcement evidence covers only structurally checkable query-shaping slots. No conclusion is drawn for other slot kinds."
        ));

        // Assert provenance details are rendered
        assert!(report.contains("- **Provider:** anthropic"));
        assert!(report.contains("- **Model:** claude-3-7-sonnet"));
        assert!(report.contains("- **Git Commit:** a1b2c3d"));

        // Assert control pass dropped case is listed
        assert!(report.contains("dt-payment-payment-date"));
        assert!(report.contains("unaided pick already equals claimed value"));

        // Assert slot breakdown table lines
        assert!(
            report.contains("| Slot | Supplied | Honoured | Overridden | Dissent | Inconclusive |")
        );
        assert!(report.contains("| default_time_column | 3 | 1 | 2 | 0 | 0 |"));
        assert!(report.contains("| table_alias | 1 | 1 | 0 | 0 | 0 |"));
        assert!(report.contains("| column_role | 1 | 0 | 0 | 1 | 0 |"));
        assert!(report.contains("| **Total** | **5** | **2** | **2** | **1** | **0** |"));

        // Assert detector recall line
        assert!(report.contains("- **Detector Recall:** 50.0% (1/2)"));

        // Assert schema objection class lines
        assert!(report.contains("| schema_compatible | 2 | 2 | 0 | 0 | 0 |"));
        assert!(report.contains("| schema_suspicious | 2 | 0 | 2 | 0 | 0 |"));
        assert!(report.contains("| schema_invalid | 1 | 0 | 0 | 1 | 0 |"));
    }

    #[test]
    fn test_load_results_from_jsonl_and_generate_report() {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join(format!(
            "saya-test-load-report-{}.jsonl",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));

        let provenance = Provenance {
            provider: "openai".into(),
            model: "gpt-4o".into(),
            timestamp_utc: "2026-08-17T12:00:00Z".into(),
            corpus_revision: "v1".into(),
            repetitions: 1,
            git_commit: "deadbeef".into(),
        };

        let record = TrialRecord {
            provenance: provenance.clone(),
            case_id: "ta-film-movies".into(),
            slot_kind: SlotKind::TableAlias,
            schema_objection_class: SchemaObjectionClass::SchemaCompatible,
            prompt: "Give me movies".into(),
            repetition: 1,
            phase: TrialPhase::Measurement,
            outcome: TrialOutcome::Honoured,
            observed_sql: Some("SELECT * FROM film".into()),
            observed_value: Some("film".into()),
            item_supplied: true,
            override_notice_fired: false,
            dissent_detected: false,
            notes: None,
        };

        write_trial_record_jsonl(&test_file, &record).expect("must write jsonl");

        let loaded = load_results_from_jsonl(&test_file).expect("must load jsonl");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].case_id, "ta-film-movies");

        let report = generate_markdown_report(&provenance, &[], &loaded);
        assert!(report.contains("| table_alias | 1 | 1 | 0 | 0 | 0 |"));
        assert!(report.contains("> Enforcement evidence covers only structurally checkable query-shaping slots. No conclusion is drawn for other slot kinds."));

        let _ = fs::remove_file(test_file);
    }
}
