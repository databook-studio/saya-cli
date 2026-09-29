//! Selecting what a manifest import maps: model and source nodes filtered by
//! `--select` globs over their names, bounded to [`MAX_SELECTED_NODES`], plus
//! the relationships tests the mapper will try to attach.

use std::path::Path;

use super::manifest::{DbtVersion, ManifestView, NodeView, probe_version, read_bounded};
use super::{DbtManifestError, MAX_SELECTED_NODES};

/// The manifest reduced to what the mapper consumes: the schema version, the
/// producing dbt version, and the selected nodes and relationships tests,
/// each list ordered by unique_id.
pub(super) struct Selected {
    pub version: DbtVersion,
    pub dbt_version: String,
    pub objects: Vec<(String, NodeView)>,
    pub relationships: Vec<(String, NodeView)>,
}

/// Reads, bounds, and selects a manifest. `select` holds glob patterns matched
/// against node names (`*` runs any sequence, `?` one character; an empty
/// slice selects everything). Only `resource_type` model and source nodes are
/// selected; seeds, snapshots, and every other node kind are out of scope.
pub(super) fn read_and_select(
    path: &Path,
    select: &[String],
) -> Result<Selected, DbtManifestError> {
    let bytes = read_bounded(path)?;
    let version = probe_version(&bytes)?;
    let manifest: ManifestView =
        serde_json::from_slice(&bytes).map_err(|_| DbtManifestError::Malformed)?;
    let dbt_version = manifest
        .metadata
        .and_then(|metadata| metadata.dbt_version)
        .filter(|version| !version.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    let mut objects: Vec<(String, NodeView)> = manifest
        .nodes
        .iter()
        .chain(manifest.sources.iter())
        .filter(|(_, node)| matches!(node.resource_type.as_deref(), Some("model" | "source")))
        .filter(|(_, node)| {
            let name = node.name.as_deref().unwrap_or("");
            select.is_empty() || select.iter().any(|pattern| glob_match(pattern, name))
        })
        .map(|(unique_id, node)| (unique_id.clone(), node.clone()))
        .collect();
    if objects.len() > MAX_SELECTED_NODES {
        return Err(DbtManifestError::TooManyNodes(objects.len()));
    }
    // `nodes` and `sources` chains two ordered maps, so order by unique_id.
    objects.sort_by(|a, b| a.0.cmp(&b.0));
    let relationships: Vec<(String, NodeView)> = manifest
        .nodes
        .iter()
        .filter(|(_, node)| node.resource_type.as_deref() == Some("test"))
        .filter(|(_, node)| {
            node.test_metadata
                .as_ref()
                .and_then(|metadata| metadata.name.as_deref())
                == Some("relationships")
        })
        .map(|(unique_id, node)| (unique_id.clone(), node.clone()))
        .collect();
    Ok(Selected {
        version,
        dbt_version,
        objects,
        relationships,
    })
}

/// Matches one dbt node name against one `--select` glob: `*` runs any
/// sequence (including empty) and `?` matches exactly one character,
/// case-sensitively. Iterative two-pointer with star backtracking.
fn glob_match(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0usize, 0usize);
    let (mut star, mut mark) = (None, 0usize);
    while ni < name.len() {
        if pi < pattern.len() && (pattern[pi] == '?' || pattern[pi] == name[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < pattern.len() && pattern[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(star) = star {
            pi = star + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    pattern[pi..].iter().all(|&c| c == '*')
}
