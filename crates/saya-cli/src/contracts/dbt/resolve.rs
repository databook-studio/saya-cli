//! Turning a dbt target reference into a logical object.
//!
//! `kwargs.to` holds a Jinja call — `ref('customers')` or
//! `source('jaffle', 'events')`. Resolution is name-only and selection-scoped:
//! the referenced name must match exactly one selected node, whose logical
//! object supplies the target's catalog, schema, and name. Nothing is followed
//! on disk, and nothing outside the selection can be named.

use std::collections::HashMap;

use saya_types::{DatabaseObjectKind, PortableObject};

use super::manifest::NodeView;

/// Selected nodes indexed by the name a `ref()` or `source()` call uses.
pub(super) type NameIndex<'a> = HashMap<&'a str, Vec<&'a NodeView>>;

/// Indexes the selected nodes by name — models and sources in separate
/// indexes, since `ref()` names models and `source()` names sources.
pub(super) fn indexes_by_name(objects: &[(String, NodeView)]) -> (NameIndex<'_>, NameIndex<'_>) {
    let mut models = NameIndex::default();
    let mut sources = NameIndex::default();
    for (_, node) in objects {
        let index = match node.resource_type.as_deref() {
            Some("model") => &mut models,
            Some("source") => &mut sources,
            _ => continue,
        };
        if let Some(name) = nonempty(node.name.as_deref()) {
            index.entry(name).or_default().push(node);
        }
    }
    (models, sources)
}

/// The logical object a node maps to: catalog and schema as the manifest
/// reported them (absent stays absent), and the name a dbt user would know —
/// the alias for models, the identifier for sources, the bare name otherwise.
pub(super) fn object_of(node: &NodeView) -> Option<PortableObject> {
    let name = nonempty(node.alias.as_deref())
        .or_else(|| nonempty(node.identifier.as_deref()))
        .or_else(|| nonempty(node.name.as_deref()))?;
    Some(PortableObject {
        catalog: nonempty(node.database.as_deref()).map(str::to_string),
        schema: nonempty(node.schema.as_deref()).map(str::to_string),
        name: name.to_string(),
        kind: DatabaseObjectKind::Table,
    })
}

/// An optional manifest string that carries content: present and not empty.
pub(super) fn nonempty(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.is_empty())
}

/// Resolves one `kwargs.to` reference against the selected nodes. Returns a
/// stable reason code when the call is not a `ref`/`source` invocation, when
/// the name matches nothing selected, or when it matches several — the caller
/// skips and counts. A `ref`'s version argument and a `source`'s first
/// (source-name) argument are not carried by this view, so they do not
/// narrow the match; a name that only matches under that narrowed reading
/// would be ambiguous here, which is the safe failure.
pub(super) fn resolve_target(
    to: &str,
    models: &NameIndex<'_>,
    sources: &NameIndex<'_>,
) -> Result<PortableObject, &'static str> {
    let (function, arguments) = parse_call(to).ok_or("malformed_target")?;
    let candidates: &[&NodeView] = match function {
        "ref" => index_for(models, arguments.first().ok_or("malformed_target")?),
        "source" => index_for(sources, arguments.get(1).ok_or("malformed_target")?),
        _ => return Err("malformed_target"),
    };
    match candidates {
        [] => Err("unresolved_target"),
        [node] => object_of(node).ok_or("unresolved_target"),
        _ => Err("ambiguous_target"),
    }
}

fn index_for<'a>(index: &'a NameIndex<'a>, name: &str) -> &'a [&'a NodeView] {
    index.get(name).map(Vec::as_slice).unwrap_or(&[])
}

/// Parses a Jinja call: optional `{{ }}` wrapping, the function name, and its
/// quoted arguments. `ref('customers')` → `("ref", ["customers"])`;
/// `source('jaffle', 'events')` → `("source", ["jaffle", "events"])`. An
/// unparseable reference is the caller's skip, never a panic.
fn parse_call(to: &str) -> Option<(&str, Vec<String>)> {
    let trimmed = to.trim();
    let inner = trimmed
        .strip_prefix("{{")
        .and_then(|rest| rest.strip_suffix("}}"))
        .unwrap_or(trimmed)
        .trim();
    let (function, arguments) = inner.split_once('(')?;
    let arguments = arguments.strip_suffix(')')?;
    Some((function.trim(), quoted_arguments(arguments)))
}

/// Extracts the single- or double-quoted strings inside a call's arguments.
fn quoted_arguments(arguments: &str) -> Vec<String> {
    let mut characters = arguments.char_indices();
    let mut quoted = Vec::new();
    while let Some((_, open)) = characters.next() {
        if open != '\'' && open != '"' {
            continue;
        }
        let mut value = String::new();
        for (_, character) in characters.by_ref() {
            if character == open {
                break;
            }
            value.push(character);
        }
        quoted.push(value);
    }
    quoted
}
