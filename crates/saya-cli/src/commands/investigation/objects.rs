//! The canonical rendering of a definition's informational `objects` field
//! (A2 decision 2): each identifier part renders bare when it needs no
//! quoting, and double-quoted — with `"` doubled — otherwise, and the parts
//! join with ".". A part that itself contains a dot (`"orders.v1"`) therefore
//! stays one object, so the rendering can never be read back as a
//! schema/table split. Save writes this form; import recomputes it from the
//! SQL and refuses a document that disagrees.

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
        .map(|parts| render_object(parts))
        .collect()
}

/// Renders one object's identifier parts, as [`canonical_objects`] renders
/// each; also the review's display form for an unresolved name.
pub(super) fn render_object(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| render_part(part))
        .collect::<Vec<_>>()
        .join(".")
}

fn render_part(part: &str) -> String {
    if is_bare(part) {
        part.to_string()
    } else {
        format!("\"{}\"", part.replace('"', "\"\""))
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
