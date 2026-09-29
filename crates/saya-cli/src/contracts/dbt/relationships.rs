//! Mapping relationships tests onto portable relationship items.
//!
//! A dbt `relationships` generic test maps to one [`PortablePayload::
//! Relationship`]: the local column is the test's `column_name`, the target
//! column rides in `kwargs.field` (older) or `kwargs.arguments.field`, and
//! the target reference in `kwargs.to` must resolve to exactly one selected
//! node. The payload goes through the validating claim constructor and
//! `from_claim`; anything they refuse is skipped and counted, never loosened.

use saya_types::{
    Cardinality, ClaimPayload, ContextItem, DatabaseObjectKind, DatabaseObjectRef, PortableObject,
    PortablePayload, ProfileIdentity,
};

use super::DbtImport;
use super::manifest::NodeView;
use super::map::origin_note;
use super::resolve::{NameIndex, nonempty, object_of, resolve_target};

/// What every mapping step shares: the selected node index by unique_id, the
/// two name indexes a `ref()`/`source()` resolves against, the producing dbt
/// version provenance notes name, and the synthetic profile identity.
pub(super) struct MappingContext<'a> {
    pub dbt_version: &'a str,
    pub by_id: &'a std::collections::HashMap<&'a str, &'a NodeView>,
    pub models: &'a NameIndex<'a>,
    pub sources: &'a NameIndex<'a>,
    pub profile: &'a ProfileIdentity,
}

pub(super) fn map_relationship(
    import: &mut DbtImport,
    unique_id: &str,
    test: &NodeView,
    context: &MappingContext<'_>,
) {
    let note = match origin_note(context.dbt_version, unique_id) {
        Ok(note) => note,
        Err(reason) => {
            import.skipped.push((unique_id.to_string(), reason));
            return;
        }
    };
    // A test attached to a node outside the selection is out of scope, not a
    // failure: nothing maps and nothing is reported.
    let Some(attached_node) = test
        .attached_node
        .as_deref()
        .and_then(|id| context.by_id.get(id).copied())
    else {
        return;
    };
    let Some(object) = object_of(attached_node) else {
        import.skipped.push((unique_id.to_string(), "missing_name"));
        return;
    };
    let Some(local_column) = nonempty(test.column_name.as_deref()) else {
        import
            .skipped
            .push((unique_id.to_string(), "missing_column"));
        return;
    };
    let Some(kwargs) = test
        .test_metadata
        .as_ref()
        .and_then(|metadata| metadata.kwargs.as_ref())
    else {
        import
            .skipped
            .push((unique_id.to_string(), "malformed_target"));
        return;
    };
    let target = match kwargs
        .to
        .as_deref()
        .map(|to| resolve_target(to, context.models, context.sources))
    {
        Some(Ok(target)) => target,
        Some(Err(reason)) => {
            import.skipped.push((unique_id.to_string(), reason));
            return;
        }
        None => {
            import
                .skipped
                .push((unique_id.to_string(), "malformed_target"));
            return;
        }
    };
    let Some(target_column) = nonempty(kwargs.field.as_deref()).or_else(|| {
        kwargs
            .arguments
            .as_ref()
            .and_then(|arguments| nonempty(arguments.field.as_deref()))
    }) else {
        import
            .skipped
            .push((unique_id.to_string(), "missing_column"));
        return;
    };
    let carried = relationship_payload(context.profile, &target, local_column, target_column);
    match carried {
        Some(payload) => import.items.push(ContextItem {
            object,
            payload,
            origin_note: Some(note),
        }),
        None => {
            import
                .skipped
                .push((unique_id.to_string(), "invalid_relationship"));
        }
    }
}

/// Builds the validated portable relationship payload, or `None` when a
/// validating constructor refuses a name. The portable `Relationship` variant
/// is `#[non_exhaustive]`, so the only path from another crate is the claim
/// constructor plus `from_claim` — which needs a `DatabaseObjectRef` and
/// strips its profile.
fn relationship_payload(
    profile: &ProfileIdentity,
    target: &PortableObject,
    local_column: &str,
    target_column: &str,
) -> Option<PortablePayload> {
    let target_ref = DatabaseObjectRef::new(
        profile.clone(),
        target.catalog.as_deref().unwrap_or(""),
        target.schema.as_deref().unwrap_or(""),
        &target.name,
        DatabaseObjectKind::Table,
    )
    .ok()?;
    ClaimPayload::relationship(
        target_ref,
        vec![local_column.to_string()],
        vec![target_column.to_string()],
        // A dbt relationships test asserts every local value exists in the
        // target: many local rows point at one target row.
        Cardinality::ManyToOne,
    )
    .ok()
    .and_then(|claim| PortablePayload::from_claim(&claim).ok())
}
