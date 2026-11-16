//! Reading and bounding a dbt manifest: the byte cap, the schema-version
//! gate, and the minimal typed view the mapper works from. Only the fields
//! the view names are read — a real manifest's configs, compiled SQL, macros,
//! and every other field are ignored wholesale, never executed or carried.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde::Deserialize;

use super::{DbtManifestError, MAX_MANIFEST_BYTES};

/// The manifest schema versions this parser reads: dbt 1.6 wrote v10, 1.7
/// wrote v11, and 1.8 through 2.0 write v12. Anything else is refused by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbtVersion {
    V10,
    V11,
    V12,
}

impl DbtVersion {
    /// Reads `metadata.dbt_schema_version` — a URL such as
    /// `https://schemas.getdbt.com/dbt/manifest/v12.json` — accepting only
    /// [`DbtVersion`]'s three versions.
    pub fn parse(schema_version: &str) -> Option<Self> {
        let stem = schema_version.rsplit('/').next()?.strip_suffix(".json")?;
        match stem {
            "v10" => Some(Self::V10),
            "v11" => Some(Self::V11),
            "v12" => Some(Self::V12),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::V10 => "v10",
            Self::V11 => "v11",
            Self::V12 => "v12",
        }
    }
}

/// The minimal node view: a real manifest's `nodes` map holds models, tests,
/// and every other node kind side by side, so one lenient struct covers all
/// of them and the mapper filters by `resource_type`. Every field is
/// optional, and every unknown field is ignored.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub(super) struct NodeView {
    pub resource_type: Option<String>,
    pub database: Option<String>,
    pub schema: Option<String>,
    pub name: Option<String>,
    pub alias: Option<String>,
    pub identifier: Option<String>,
    pub description: Option<String>,
    pub columns: BTreeMap<String, ColumnView>,
    /// Relationship-test-only fields, present on `resource_type: "test"` nodes.
    pub attached_node: Option<String>,
    pub column_name: Option<String>,
    pub test_metadata: Option<TestMetadata>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub(super) struct ColumnView {
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub(super) struct TestMetadata {
    pub name: Option<String>,
    pub kwargs: Option<Kwargs>,
}

/// The fields a `relationships` test's kwargs may carry, across the older
/// `field` shape and the newer `arguments.field` shape; everything else in
/// kwargs is ignored.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub(super) struct Kwargs {
    pub to: Option<String>,
    pub field: Option<String>,
    pub arguments: Option<KwargsArguments>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub(super) struct KwargsArguments {
    pub field: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct ManifestView {
    pub metadata: Option<MetadataView>,
    pub nodes: BTreeMap<String, NodeView>,
    pub sources: BTreeMap<String, NodeView>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct MetadataView {
    pub dbt_schema_version: Option<String>,
    pub dbt_version: Option<String>,
}

/// Reads the manifest file, refusing anything over [`MAX_MANIFEST_BYTES`]
/// before a byte is parsed: the declared size is checked first, and the read
/// itself is capped so a file that grows mid-read cannot exceed the bound.
pub(super) fn read_bounded(path: &Path) -> Result<Vec<u8>, DbtManifestError> {
    let file = File::open(path)?;
    let declared = file.metadata()?.len();
    if declared > MAX_MANIFEST_BYTES as u64 {
        return Err(DbtManifestError::Oversize(declared as usize));
    }
    let mut bytes = Vec::new();
    file.take(MAX_MANIFEST_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(DbtManifestError::Oversize(bytes.len()));
    }
    Ok(bytes)
}

/// Probes only `metadata.dbt_schema_version`, so an unsupported version fails
/// by name before the full parse runs.
pub(super) fn probe_version(bytes: &[u8]) -> Result<DbtVersion, DbtManifestError> {
    #[derive(Deserialize)]
    struct Probe {
        metadata: Option<MetadataView>,
    }
    let probe: Probe = serde_json::from_slice(bytes).map_err(|_| DbtManifestError::Malformed)?;
    let version = probe
        .metadata
        .and_then(|metadata| metadata.dbt_schema_version)
        .ok_or(DbtManifestError::NotAManifest)?;
    DbtVersion::parse(&version).ok_or(DbtManifestError::UnsupportedVersion(version))
}
