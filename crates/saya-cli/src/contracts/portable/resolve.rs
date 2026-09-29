//! Resolving a portable logical object against one profile's schema tree.
//!
//! Resolution is case-insensitive and refuses to pick among several
//! candidates — the review-gate identity rule the fingerprint review uses. The
//! dialect-aware identity module is `pub(super)` to the investigation
//! fingerprint, so this portable path uses the spec's sanctioned fallback
//! (case-insensitive, several candidates → ambiguous); portable objects carry
//! no quoting, so dialect folding could not be applied soundly anyway. A
//! resolved candidate keeps the tree's spelling: it is the object the schema
//! cache names, and every later classification compares against that spelling.

use saya_types::{DatabaseObjectRef, PortableObject, ProfileIdentity, SchemaTree, Table};

pub(super) enum Resolution<'a> {
    /// Exactly one object matched; `catalog`/`schema` carry the tree's
    /// spelling and `table` the live table to fingerprint and bind against.
    Resolved {
        catalog: String,
        schema: String,
        table: &'a Table,
    },
    Missing,
    Ambiguous,
}

pub(super) fn resolve_object<'a>(tree: &'a SchemaTree, object: &PortableObject) -> Resolution<'a> {
    let eq = |left: &str, right: &str| left.eq_ignore_ascii_case(right);
    let matches: Vec<(&saya_types::Database, &saya_types::Schema, &Table)> = tree
        .databases
        .iter()
        .filter(|db| object.catalog.as_deref().is_none_or(|c| eq(&db.name, c)))
        .flat_map(|db| db.schemas.iter().map(move |schema| (db, schema)))
        .filter(|(_, schema)| object.schema.as_deref().is_none_or(|s| eq(&schema.name, s)))
        .flat_map(|(db, schema)| schema.tables.iter().map(move |table| (db, schema, table)))
        .filter(|(_, _, table)| eq(&table.name, &object.name))
        .collect();
    match matches.as_slice() {
        [] => Resolution::Missing,
        [(db, schema, table)] => Resolution::Resolved {
            catalog: db.name.clone(),
            schema: schema.name.clone(),
            table,
        },
        _ => Resolution::Ambiguous,
    }
}

/// The `DatabaseObjectRef` a resolved target becomes, for the target-bearing
/// payload constructors; `None` when the target does not resolve.
pub(super) fn resolve_ref(
    tree: &SchemaTree,
    identity: &ProfileIdentity,
    object: &PortableObject,
) -> Option<DatabaseObjectRef> {
    let (catalog, schema) = match resolve_object(tree, object) {
        Resolution::Resolved {
            catalog, schema, ..
        } => (catalog, schema),
        _ => return None,
    };
    DatabaseObjectRef::new(identity.clone(), catalog, schema, &object.name, object.kind).ok()
}

/// The label a report shows for an unmapped item: its own spelling.
pub(super) fn object_label(object: &PortableObject) -> String {
    let mut label = String::new();
    if let Some(catalog) = &object.catalog {
        label.push_str(catalog);
        label.push('.');
    }
    if let Some(schema) = &object.schema {
        label.push_str(schema);
        label.push('.');
    }
    label.push_str(&object.name);
    label
}
