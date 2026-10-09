//! Tests for the bounded dbt manifest parser (B3a).
//!
//! Bounds and schema versions are enforced before anything parses; untrusted
//! manifest text never becomes anything but validated claim data; and every
//! mapped item carries provenance. Fixtures under `fixtures/` are synthetic —
//! manifest-shaped JSON written by hand, no dbt output.

use std::fs::File;
use std::path::{Path, PathBuf};

use super::{
    DbtImport, DbtManifestError, DbtVersion, MAX_MANIFEST_BYTES, MAX_SELECTED_NODES,
    parse_dbt_manifest,
};
use saya_types::{Cardinality, ClaimPayload, ContextItem, DatabaseObjectKind, PortablePayload};

const V12: &str = include_str!("fixtures/v12_manifest.json");
const V11: &str = include_str!("fixtures/v11_manifest.json");
const V10: &str = include_str!("fixtures/v10_manifest.json");
const INJECTION: &str = include_str!("fixtures/injection_manifest.json");

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn temp_root(label: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("saya-dbt-{label}-{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn manifest_file(label: &str, contents: &str) -> PathBuf {
    let path = temp_root(label).join("manifest.json");
    std::fs::write(&path, contents).unwrap();
    path
}

fn parse(path: &Path, select: &[&str]) -> Result<DbtImport, DbtManifestError> {
    let select = select
        .iter()
        .map(|pattern| (*pattern).to_string())
        .collect::<Vec<_>>();
    parse_dbt_manifest(path, &select)
}

fn parse_default(path: &Path) -> DbtImport {
    parse(path, &[]).expect("the fixture manifest parses")
}

fn nodes_manifest(label: &str, count: usize) -> PathBuf {
    let mut nodes = serde_json::Map::new();
    for index in 0..count {
        nodes.insert(
            format!("model.jaffle.m{index}"),
            serde_json::json!({
                "resource_type": "model",
                "name": format!("m{index}"),
                "database": "analytics",
                "schema": "core"
            }),
        );
    }
    let manifest = serde_json::json!({
        "metadata": {
            "dbt_schema_version": "https://schemas.getdbt.com/dbt/manifest/v12.json",
            "dbt_version": "1.8.6"
        },
        "nodes": nodes
    });
    manifest_file(label, &manifest.to_string())
}

/// The relationship item whose single local column is `local`.
fn relationship_local<'a>(import: &'a DbtImport, local: &str) -> Option<&'a ContextItem> {
    import.items.iter().find(|item| {
        matches!(
            &item.payload,
            PortablePayload::Relationship { local_columns, .. }
                if local_columns.len() == 1 && local_columns[0] == local
        )
    })
}

// ---------------------------------------------------------------------------
// bounds, versions, and selection
// ---------------------------------------------------------------------------

include!("tests/bounds_selection.rs");

// ---------------------------------------------------------------------------
// untrusted text is data, never instruction or executed code
// ---------------------------------------------------------------------------

include!("tests/untrusted_manifest.rs");

// ---------------------------------------------------------------------------
// mapping: objects, descriptions, and relationships
// ---------------------------------------------------------------------------

include!("tests/mapping_provenance.rs");
