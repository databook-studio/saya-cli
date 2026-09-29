//! Mapping a selected dbt manifest onto portable context items.
//!
//! Every payload goes through the existing validating constructors in
//! `saya-types`; anything they refuse is skipped and counted with a stable
//! reason code, never loosened. Item text is data — untrusted manifest text
//! becomes a claim's content exactly as validated, and never anything else.

use std::collections::HashMap;

use saya_types::{
    ClaimPayload, ContextItem, MAX_ORIGIN_NOTE_BYTES, PortableObject, PortablePayload,
    ProfileIdentity,
};

use super::DbtImport;
use super::manifest::NodeView;
use super::relationships::{MappingContext, map_relationship};
use super::resolve::{indexes_by_name, object_of};
use super::select::Selected;

/// Maps a selected manifest: one pass over the selected model/source nodes
/// emitting their description and column-description items, then one pass over
/// the relationships tests. Skips carry stable reason codes, one per refused
/// item: `missing_name` (the node has no usable name),
/// `origin_note_oversize` / `origin_note_invalid` (provenance the item bounds
/// refuse), `table_description_invalid` / `column_description_invalid` (text
/// the claim constructor refuses), and — from the relationship mapper —
/// `malformed_target` / `unresolved_target` / `ambiguous_target`,
/// `missing_column`, and `invalid_relationship`.
pub(super) fn map(selected: Selected) -> DbtImport {
    let mut import = DbtImport {
        items: Vec::new(),
        skipped: Vec::new(),
        version: selected.version,
    };
    let by_id: HashMap<&str, &NodeView> = selected
        .objects
        .iter()
        .map(|(unique_id, node)| (unique_id.as_str(), node))
        .collect();
    let (models, sources) = indexes_by_name(&selected.objects);
    let profile = profile_identity();
    let context = MappingContext {
        dbt_version: &selected.dbt_version,
        by_id: &by_id,
        models: &models,
        sources: &sources,
        profile: &profile,
    };
    for (unique_id, node) in &selected.objects {
        let note = match origin_note(&selected.dbt_version, unique_id) {
            Ok(note) => note,
            Err(reason) => {
                import.skipped.push((unique_id.clone(), reason));
                continue;
            }
        };
        let Some(object) = object_of(node) else {
            import.skipped.push((unique_id.clone(), "missing_name"));
            continue;
        };
        emit_description(
            &mut import,
            &object,
            &note,
            unique_id,
            None,
            node.description.as_deref(),
        );
        for (column, view) in &node.columns {
            emit_description(
                &mut import,
                &object,
                &note,
                unique_id,
                Some(column),
                view.description.as_deref(),
            );
        }
    }
    for (unique_id, test) in &selected.relationships {
        map_relationship(&mut import, unique_id, test, &context);
    }
    import
}

/// The synthetic profile identity the relationship mapper needs; see
/// `relationships::relationship_payload`.
fn profile_identity() -> ProfileIdentity {
    ProfileIdentity::parse(&format!("p-{}", "0".repeat(64)))
        .expect("64 zeros form a valid profile identity")
}

/// Emits one description claim — a table description when `column` is `None`,
/// a column description otherwise — as a context item, or records the skip. A
/// whitespace-only description carries no content and maps to nothing; text a
/// validating constructor refuses is skipped and counted, never loosened.
fn emit_description(
    import: &mut DbtImport,
    object: &PortableObject,
    note: &str,
    unique_id: &str,
    column: Option<&str>,
    text: Option<&str>,
) {
    let Some(text) = text.map(str::trim).filter(|text| !text.is_empty()) else {
        return;
    };
    let reason = if column.is_some() {
        "column_description_invalid"
    } else {
        "table_description_invalid"
    };
    let carried = match column {
        Some(column) => ClaimPayload::column_description(column, text),
        None => ClaimPayload::table_description(text),
    }
    .ok()
    .and_then(|claim| PortablePayload::from_claim(&claim).ok());
    match carried {
        Some(payload) => import.items.push(ContextItem {
            object: object.clone(),
            payload,
            origin_note: Some(note.to_string()),
        }),
        None => import.skipped.push((unique_id.to_string(), reason)),
    }
}

/// Builds the provenance note every mapped item carries: `dbt <dbt_version>
/// <unique_id>`. Both parts are untrusted manifest text, so the note is
/// bounded and control-char-checked exactly like the item validation that
/// would run on the way into a document — a note that cannot pass is a skip,
/// not a loosened bound. A missing `dbt_version` says so rather than inventing
/// one.
pub(super) fn origin_note(dbt_version: &str, unique_id: &str) -> Result<String, &'static str> {
    if dbt_version.len() > MAX_ORIGIN_NOTE_BYTES || unique_id.len() > MAX_ORIGIN_NOTE_BYTES {
        return Err("origin_note_oversize");
    }
    let note = format!("dbt {dbt_version} {unique_id}");
    if note.len() > MAX_ORIGIN_NOTE_BYTES {
        return Err("origin_note_oversize");
    }
    if note.chars().any(char::is_control) {
        return Err("origin_note_invalid");
    }
    Ok(note)
}
