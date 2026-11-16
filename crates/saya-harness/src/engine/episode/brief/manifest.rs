//! The brief's workspace manifest walk: names, sizes, digests — never bulk
//! contents.

use crate::workspace::Workspace;
use crate::workspace::manifest::{self, ManifestEntry};

use crate::engine::episode::ManifestBounds;

/// Walks the workspace for the brief: names, sizes, digests — never bulk
/// contents. An empty workspace is an empty manifest.
pub(crate) fn manifest(
    workspace: &Workspace,
    bounds: &ManifestBounds,
) -> Result<Vec<ManifestEntry>, crate::HarnessError> {
    manifest::build(workspace, bounds.max_files, bounds.max_file_bytes)
}
