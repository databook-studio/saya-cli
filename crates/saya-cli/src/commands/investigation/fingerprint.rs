//! The replay-side schema fingerprint (S7, invariant 2): resolves the
//! definition's referenced objects against the live schema tree and folds
//! their per-table digests into one combined value, so a review goes stale
//! the moment any referenced object's columns change.
//!
//! Resolution rules, decided here (D4) and stable across machines: a
//! fully-qualified name (`catalog.schema.object`) resolves exactly; a
//! two-part name (`schema.object`) resolves with the first part as the
//! schema, scanning the tree's databases, because the tree's top-level
//! database name is engine-specific (SQLite names it after the file, MySQL
//! uses a fixed label while the connected database is the schema); a
//! one-part name — the flat dialects' only form — scans every database and
//! schema. The first match in tree order wins, so resolution is
//! deterministic for a given tree.
//!
//! The digest is SHA-256 over the length-prefixed, lexicographically sorted
//! list of `object_name ":" table_fingerprint` entries; an object that does
//! not resolve contributes `object_name ":missing"` instead of vanishing,
//! so a dropped table invalidates the review too. Zero objects mean there
//! is nothing to bind: the result is `None` and no schema is needed.

use saya_types::{DatabaseObjectKind, SchemaFingerprint, SchemaTree, Table};
use sha2::{Digest, Sha256};

/// The combined fingerprint of the referenced objects against `tree`, or
/// `None` when the definition references no objects.
pub(super) fn combined_fingerprint(tree: &SchemaTree, objects: &[String]) -> Option<String> {
    if objects.is_empty() {
        return None;
    }
    let mut entries: Vec<String> = objects
        .iter()
        .map(|object| {
            let fingerprint =
                resolve(tree, object).map_or_else(|| "missing".to_string(), table_fingerprint);
            format!("{object}:{fingerprint}")
        })
        .collect();
    entries.sort();
    let mut hash = Sha256::new();
    for entry in &entries {
        hash.update((entry.len() as u64).to_be_bytes());
        hash.update(entry.as_bytes());
    }
    Some(
        hash.finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}

/// Resolves one dotted object name (`a`, `s.a`, or `c.s.a`) against the
/// tree, per the module's resolution rules.
fn resolve<'a>(tree: &'a SchemaTree, object: &str) -> Option<&'a Table> {
    let parts: Vec<&str> = object.split('.').collect();
    match parts.as_slice() {
        [table] => tree
            .databases
            .iter()
            .flat_map(|db| db.schemas.iter())
            .find_map(|schema| {
                schema
                    .tables
                    .iter()
                    .find(|t| t.name.eq_ignore_ascii_case(table))
            }),
        [schema, table] => tree.databases.iter().find_map(|db| {
            db.schemas
                .iter()
                .find(|s| s.name.eq_ignore_ascii_case(schema))
                .and_then(|s| s.tables.iter().find(|t| t.name.eq_ignore_ascii_case(table)))
        }),
        [catalog, schema, table] => tree.find_table(catalog, schema, table),
        _ => None,
    }
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
