use super::ResolutionError;
use saya_types::{
    DatabaseObjectKind, DatabaseObjectRef, ProfileIdentity, SchemaBinding, SchemaTree,
};

/// Refuses a proposal whose schema binding names a column the resolved object
/// does not have.
///
/// Observed columns come from result sets and SQL text, where a SELECT alias
/// referenced in ORDER BY or HAVING and a join's output columns can both end
/// up attributed to a table that never owned them. A fact about a column that
/// does not exist would be stored as if it were real, so the schema — not the
/// observation — decides existence. Only presence is checked here: a semantic
/// type mismatch is a staleness question for the validity reconciler, not a
/// reason to refuse a fact on a dynamically typed backend.
pub(super) fn validate_binding_columns(
    schema_tree: &SchemaTree,
    object: &DatabaseObjectRef,
    binding: &SchemaBinding,
) -> Result<(), ResolutionError> {
    // The object was resolved from this very tree, so the lookup cannot miss;
    // a miss is treated as the resolution failure it would mean.
    let Some(table) = schema_tree.find_table(object.catalog(), object.schema(), object.object())
    else {
        return Err(ResolutionError::UnresolvableObject(object.qualified_name()));
    };

    let missing =
        |column: &str| ResolutionError::UnknownColumn(object.qualified_name(), column.to_string());

    match binding {
        // A table-level fact names no column, so there is nothing to refuse.
        SchemaBinding::Table => Ok(()),
        SchemaBinding::Column { column, .. } => {
            let exists = table
                .columns
                .iter()
                .any(|c| c.name.eq_ignore_ascii_case(column));
            if exists { Ok(()) } else { Err(missing(column)) }
        }
        // A multi-column fact (a join rule's local keys, a metric's underlying
        // columns) is refused when any one of its columns is absent: a fact
        // about a column that does not exist would be stored as if it were
        // real, and the schema — not the observation — decides existence.
        SchemaBinding::Columns { columns } => {
            for column in columns {
                if !table
                    .columns
                    .iter()
                    .any(|c| c.name.eq_ignore_ascii_case(column))
                {
                    return Err(missing(column));
                }
            }
            Ok(())
        }
    }
}

/// Resolves an object name (1-, 2-, or 3-part) against a profile's real `SchemaTree`.
/// Refuses to invent or fabricate catalog/schema names.
pub(super) fn resolve_database_object(
    profile: &ProfileIdentity,
    schema_tree: &SchemaTree,
    qualified_name: &str,
) -> Result<DatabaseObjectRef, ResolutionError> {
    let parts: Vec<&str> = qualified_name
        .split('.')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();

    let mut matches = Vec::new();
    match parts.as_slice() {
        [cat, sch, obj] => {
            for db in &schema_tree.databases {
                if db.name.eq_ignore_ascii_case(cat) {
                    for s in &db.schemas {
                        if s.name.eq_ignore_ascii_case(sch) {
                            for t in &s.tables {
                                if t.name.eq_ignore_ascii_case(obj) {
                                    matches.push((
                                        db.name.as_str(),
                                        s.name.as_str(),
                                        t.name.as_str(),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        [sch, obj] => {
            for db in &schema_tree.databases {
                for s in &db.schemas {
                    if s.name.eq_ignore_ascii_case(sch) {
                        for t in &s.tables {
                            if t.name.eq_ignore_ascii_case(obj) {
                                matches.push((db.name.as_str(), s.name.as_str(), t.name.as_str()));
                            }
                        }
                    }
                }
            }
        }
        [obj] => {
            for db in &schema_tree.databases {
                for s in &db.schemas {
                    for t in &s.tables {
                        if t.name.eq_ignore_ascii_case(obj) {
                            matches.push((db.name.as_str(), s.name.as_str(), t.name.as_str()));
                        }
                    }
                }
            }
        }
        _ => {
            return Err(ResolutionError::InvalidObjectRef(
                qualified_name.to_string(),
                "name must have 1, 2, or 3 parts".to_string(),
            ));
        }
    }

    matches.sort_unstable();
    matches.dedup();

    match matches.as_slice() {
        [(cat, sch, obj)] => {
            DatabaseObjectRef::new(profile.clone(), *cat, *sch, *obj, DatabaseObjectKind::Table)
                .map_err(|e| {
                    ResolutionError::InvalidObjectRef(qualified_name.to_string(), e.to_string())
                })
        }
        [] => Err(ResolutionError::UnresolvableObject(
            qualified_name.to_string(),
        )),
        _ => Err(ResolutionError::AmbiguousObject(qualified_name.to_string())),
    }
}
