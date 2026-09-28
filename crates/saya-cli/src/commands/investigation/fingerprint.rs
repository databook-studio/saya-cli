//! The replay-side schema review (S7 invariant 2, A2): the definition's SQL —
//! recomputed on every run through `sql_references`, never the stored
//! `objects` field — is the authority for what a review covers. Each
//! referenced object must resolve to exactly one live table under the
//! dialect's identifier rules (`identity`); a missing, ambiguous, or
//! unmodellable dependency makes the whole review unverifiable, which the
//! run refuses until `--revalidate`.
//!
//! Resolution rules, stable across machines (re-audit R1): how a name as
//! written — bare or quoted, part by part — compares with the tree's stored
//! names is decided per dialect in [`identity`], and the spelling never
//! picks among several candidates. A fully-qualified name
//! (`catalog.schema.object`) resolves within the named database and schema;
//! a two-part name (`schema.object`) resolves with the first part as the
//! schema, scanning the tree's databases, because the tree's top-level
//! database name is engine-specific (SQLite names it after the file, MySQL
//! uses a fixed label while the connected database is the schema); a
//! one-part name — the flat dialects' only form — scans every database and
//! schema. The parts arrive as the SQL wrote them and are never re-split,
//! so `"orders.v1"` is one object, not `schema orders` + `table v1`.
//!
//! The digest is SHA-256 over the length-prefixed, lexicographically sorted
//! list of `object ":" table_fingerprint` entries. It is produced only for a
//! complete analysis, so no missing-name constant can ever be bound as
//! review evidence.

use saya_connectors::SqlReferences;
use saya_types::{DatabaseObjectKind, SchemaFingerprint, SchemaTree, SqlDialect, Table};
use sha2::{Digest, Sha256};

use super::objects::render_object;

mod identity;

/// The reason a review cannot be verified: the SQL's dependency analysis is
/// absent or partial, so an object may be missing from the list.
const ANALYSIS_INCOMPLETE: &str = "dependency analysis incomplete";

/// The reason a review cannot be verified: no identifier rule covers the
/// dialect, so resolution could only guess.
const DIALECT_UNDEFINED: &str = "no identifier rules for this dialect";

/// The verdict of the schema review for one replay (A2 decisions 3–4).
pub(super) enum Analysis {
    /// Every referenced object resolved (or the SQL references none —
    /// `SELECT 1`): the payload is the combined fingerprint, `None` when
    /// there is nothing to bind.
    Complete(Option<String>),
    /// The review cannot cover the run; the payload names every reason.
    Unverifiable(String),
}

/// Whether the review needs the live schema tree: only a clean, non-partial
/// analysis with at least one referenced object does.
pub(super) fn needs_schema(references: Option<&SqlReferences>) -> bool {
    references.is_some_and(|references| !references.objects.is_empty() && !references.partial)
}

/// Reviews the SQL's referenced objects against `tree` — fetched only when
/// [`needs_schema`] says so. `references` is `None` when the SQL does not
/// parse; `partial` marks a parsed statement with unmodelled constructs.
pub(super) fn analyze(
    tree: Option<&SchemaTree>,
    dialect: SqlDialect,
    references: Option<&SqlReferences>,
) -> Analysis {
    let Some(references) = references.filter(|references| !references.partial) else {
        return Analysis::Unverifiable(ANALYSIS_INCOMPLETE.to_string());
    };
    if references.objects.is_empty() {
        return Analysis::Complete(None);
    }
    let Some(tree) = tree else {
        return Analysis::Unverifiable(ANALYSIS_INCOMPLETE.to_string());
    };
    // Quoting is parallel to `objects` by contract; a value that breaks the
    // shape cannot be resolved honestly, so it fails closed.
    if references.object_quoting.len() != references.objects.len() {
        return Analysis::Unverifiable(ANALYSIS_INCOMPLETE.to_string());
    }
    let mut problems = Vec::new();
    let mut entries = Vec::new();
    for (parts, quoting) in references.objects.iter().zip(&references.object_quoting) {
        match identity::resolve(tree, dialect, parts, quoting) {
            identity::Resolution::Resolved(table) => {
                entries.push((render_object(parts, quoting), table_fingerprint(table)));
            }
            identity::Resolution::Missing => {
                problems.push(format!("table \"{}\" not found", parts.join(".")));
            }
            identity::Resolution::Ambiguous => {
                problems.push(format!("ambiguous name \"{}\"", parts.join(".")));
            }
            identity::Resolution::NoRules => {
                return Analysis::Unverifiable(DIALECT_UNDEFINED.to_string());
            }
        }
    }
    if !problems.is_empty() {
        return Analysis::Unverifiable(problems.join("; "));
    }
    Analysis::Complete(Some(fold(&entries)))
}

/// The combined digest over the resolved entries, in the same
/// length-prefixed, lexicographically sorted scheme as before, over the
/// canonically rendered object names.
fn fold(entries: &[(String, String)]) -> String {
    let mut lines: Vec<String> = entries
        .iter()
        .map(|(object, fingerprint)| format!("{object}:{fingerprint}"))
        .collect();
    lines.sort();
    let mut hash = Sha256::new();
    for line in &lines {
        hash.update((line.len() as u64).to_be_bytes());
        hash.update(line.as_bytes());
    }
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The per-table fingerprint of a resolved object, as the run binding and
/// the evidence record it. The schema tree mixes tables and views under
/// `Schema::tables` (the SQLite discovery reads both), and the definition
/// records no kind, so every referenced object is digested as a table —
/// the same rule on both sides of a comparison, which is all staleness
/// detection needs.
pub(super) fn table_fingerprint(table: &Table) -> String {
    SchemaFingerprint::of_table(DatabaseObjectKind::Table, table)
        .as_str()
        .to_owned()
}

#[cfg(test)]
#[path = "fingerprint_tests.rs"]
mod tests;
