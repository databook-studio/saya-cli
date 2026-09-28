//! Per-dialect identifier identity rules for the schema review (re-audit
//! R1, decisions 2–3): how a referenced name as written — bare or quoted,
//! part by part — is compared with the schema tree's stored names, so the
//! review fingerprints exactly the table the engine will execute against
//! and never a same-spelled-but-different one.
//!
//! PostgreSQL folds unquoted parts to ASCII lower case and Snowflake to
//! ASCII upper case; a quoted part is never folded, and the folded form
//! must match a tree name exactly. ClickHouse and BigQuery match exactly
//! whatever the quoting. SQLite, DuckDB, and MySQL leave case behaviour to
//! the engine or server settings, so they compare case-insensitively and
//! refuse when more than one name matches — the spelling never breaks a
//! tie.

use saya_types::{SchemaTree, SqlDialect, Table};

/// The verdict of resolving one referenced object against the tree.
pub(super) enum Resolution<'a> {
    /// Exactly one table matched under the dialect's identity rules.
    Resolved(&'a Table),
    Missing,
    /// More than one table matched: the spelling never picks among
    /// candidates, so the review cannot verify this dependency.
    Ambiguous,
    /// No identity rule covers the dialect (`SqlDialect` is
    /// `#[non_exhaustive]`): resolution could only guess.
    NoRules,
}

/// Resolves one object's parts — as written, with per-part quoting — against
/// the tree under `dialect`'s rules. Exactly one candidate resolves; none is
/// missing and several are ambiguous, never resolved by spelling.
pub(super) fn resolve<'a>(
    tree: &'a SchemaTree,
    dialect: SqlDialect,
    parts: &[String],
    quoting: &[bool],
) -> Resolution<'a> {
    let Some(rules) = rules(dialect) else {
        return Resolution::NoRules;
    };
    let exact = matches!(rules, Rules::Folded(_));
    let candidates = matching(tree, &normalized(&rules, parts, quoting), exact);
    match candidates.len() {
        1 => Resolution::Resolved(candidates[0]),
        0 => Resolution::Missing,
        _ => Resolution::Ambiguous,
    }
}

/// How a dialect's identifiers compare with the tree's stored names.
enum Rules {
    /// Unquoted parts fold before an exact, case-sensitive comparison;
    /// quoted parts compare exactly as written.
    Folded(Fold),
    /// Case behaviour is engine- or setting-dependent: compare
    /// case-insensitively and let several matches fail closed.
    CaseInsensitive,
}

/// The ASCII case an unquoted part folds to.
#[derive(Clone, Copy)]
enum Fold {
    Lower,
    Upper,
    /// The dialect folds nothing: unquoted parts compare as written, still
    /// exactly.
    Identity,
}

impl Fold {
    fn apply(self, part: &str) -> String {
        match self {
            Fold::Lower => part.to_ascii_lowercase(),
            Fold::Upper => part.to_ascii_uppercase(),
            Fold::Identity => part.to_owned(),
        }
    }
}

/// The rules for `dialect`; `None` when none is defined — `SqlDialect` is
/// `#[non_exhaustive]`, so a dialect added before this match is extended
/// must make the review unverifiable, never guess.
fn rules(dialect: SqlDialect) -> Option<Rules> {
    match dialect {
        SqlDialect::Postgres => Some(Rules::Folded(Fold::Lower)),
        SqlDialect::Snowflake => Some(Rules::Folded(Fold::Upper)),
        SqlDialect::ClickHouse | SqlDialect::BigQuery => Some(Rules::Folded(Fold::Identity)),
        SqlDialect::Sqlite | SqlDialect::DuckDb | SqlDialect::Mysql => Some(Rules::CaseInsensitive),
        _ => None,
    }
}

/// Each part as the engine will read it: under a fold rule, unquoted parts
/// fold and quoted parts pass through as written; under the case-insensitive
/// rule, parts stay as written. A missing quoting entry counts as written —
/// an unknown quoting must not invent a fold.
fn normalized(rules: &Rules, parts: &[String], quoting: &[bool]) -> Vec<String> {
    match rules {
        Rules::Folded(fold) => parts
            .iter()
            .enumerate()
            .map(|(index, part)| match quoting.get(index) {
                Some(false) => fold.apply(part),
                _ => part.clone(),
            })
            .collect(),
        Rules::CaseInsensitive => parts.to_vec(),
    }
}

/// Every table whose qualifier and name match `parts` — exactly under a fold
/// rule, case-insensitively otherwise — within the scope the qualifier
/// defines.
fn matching<'a>(tree: &'a SchemaTree, parts: &[String], exact: bool) -> Vec<&'a Table> {
    let eq = |left: &str, right: &str| {
        if exact {
            left == right
        } else {
            left.eq_ignore_ascii_case(right)
        }
    };
    match parts {
        [asked_table] => tree
            .databases
            .iter()
            .flat_map(|db| db.schemas.iter())
            .flat_map(|schema| schema.tables.iter())
            .filter(|t| eq(&t.name, asked_table))
            .collect(),
        [asked_schema, asked_table] => tree
            .databases
            .iter()
            .flat_map(|db| db.schemas.iter())
            .filter(|s| eq(&s.name, asked_schema))
            .flat_map(|s| s.tables.iter())
            .filter(|t| eq(&t.name, asked_table))
            .collect(),
        [asked_catalog, asked_schema, asked_table] => tree
            .databases
            .iter()
            .filter(|db| eq(&db.name, asked_catalog))
            .flat_map(|db| db.schemas.iter())
            .filter(|s| eq(&s.name, asked_schema))
            .flat_map(|s| s.tables.iter())
            .filter(|t| eq(&t.name, asked_table))
            .collect(),
        _ => Vec::new(),
    }
}
