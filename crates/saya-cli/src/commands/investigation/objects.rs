//! The canonical rendering of a definition's informational `objects` field
//! (A2 decision 2, re-audit R1 decision 4): a part the SQL quoted renders
//! double-quoted — with `"` doubled — so `"Orders"` and `Orders` render
//! differently, and an unquoted part renders bare as written; a part that
//! could not be read back bare (one holding a dot) is quoted regardless, so
//! the rendering can never be read back as a schema/table split. Save writes
//! this form; import recomputes it from the SQL and refuses a document that
//! disagrees.

use saya_connectors::sql_references;
use saya_types::SqlDialect;

/// The objects `sql` references, canonically rendered in first-seen order;
/// empty when the SQL does not parse (the caller's SQL gate already refused
/// that case).
pub(super) fn canonical_objects(sql: &str, dialect: SqlDialect) -> Vec<String> {
    let Some(references) = sql_references(sql, dialect) else {
        return Vec::new();
    };
    references
        .objects
        .iter()
        .zip(&references.object_quoting)
        .map(|(parts, quoting)| render_object(parts, quoting))
        .collect()
}

/// Renders one object's identifier parts, as [`canonical_objects`] renders
/// each; also the review's display form for a resolved object.
pub(super) fn render_object(parts: &[String], quoting: &[bool]) -> String {
    parts
        .iter()
        .enumerate()
        .map(|(index, part)| render_part(part, quoting.get(index).copied().unwrap_or(false)))
        .collect::<Vec<_>>()
        .join(".")
}

fn render_part(part: &str, quoted_in_sql: bool) -> String {
    if quoted_in_sql || !is_bare(part) {
        format!("\"{}\"", part.replace('"', "\"\""))
    } else {
        part.to_string()
    }
}

/// True for `[A-Za-z_][A-Za-z0-9_$]*`: an identifier no dialect needs quoted.
fn is_bare(part: &str) -> bool {
    let mut chars = part.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

#[cfg(test)]
#[path = "objects_tests.rs"]
mod tests;
