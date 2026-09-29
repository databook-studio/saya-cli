//! Managing staged file sources: `--list` shows what is staged under the
//! files root, `--cleanup` removes snapshots — only directories holding a
//! valid staged source's metadata, never anything else under the root.

use crate::{commands, render::RenderFormat};
use std::{fs, path::Path};

use super::{
    format_time, human_bytes,
    snapshot::{SnapshotMeta, valid_snapshot},
};
use std::path::PathBuf;

/// Every valid staged snapshot under `root`, as `(directory, metadata)`.
/// Foreign, malformed, or dot-prefixed entries are invisible to it.
fn collect(root: &Path) -> Vec<(PathBuf, SnapshotMeta)> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found: Vec<(PathBuf, SnapshotMeta)> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|entry| {
            let dir = entry.path();
            valid_snapshot(&dir).map(|meta| (dir, meta))
        })
        .collect();
    found.sort_by(|left, right| {
        right
            .1
            .staged_unix_ms
            .cmp(&left.1.staged_unix_ms)
            .then_with(|| left.1.sha256.cmp(&right.1.sha256))
    });
    found
}

pub(super) fn list(root: &Path, format: RenderFormat) -> Result<i32, Box<dyn std::error::Error>> {
    let snapshots = collect(root);
    let message = if snapshots.is_empty() {
        "No staged file sources.".to_owned()
    } else {
        let mut lines = vec!["Staged file sources (newest first):".to_owned()];
        for (dir, meta) in &snapshots {
            lines.push(format!(
                "  {}  {}  {} rows  {}  {}",
                &meta.sha256[..12],
                meta.file_name,
                meta.rows,
                human_bytes(meta.bytes),
                format_time(meta.staged_unix_ms),
            ));
            lines.push(format!("    {}", dir.join("source.duckdb").display()));
        }
        lines.join("\n")
    };
    commands::result(message, format)
}

pub(super) fn cleanup(
    root: &Path,
    target: &str,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    if target.is_empty() {
        return Err("the --cleanup target is empty: pass `all` or a SHA-256 prefix".into());
    }
    let snapshots = collect(root);
    let chosen: Vec<&(PathBuf, SnapshotMeta)> = if target == "all" {
        snapshots.iter().collect()
    } else {
        snapshots
            .iter()
            .filter(|(_, meta)| meta.sha256.starts_with(target))
            .collect()
    };
    if target != "all" && chosen.len() > 1 {
        let matches = chosen
            .iter()
            .map(|(dir, meta)| {
                format!(
                    "{} ({})",
                    dir.file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("?"),
                    meta.file_name
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let same_content = chosen
            .iter()
            .all(|(_, meta)| meta.sha256 == chosen[0].1.sha256);
        let advice = if same_content {
            "these share one content hash and differ only in parse options; `all` removes them all"
        } else {
            "use a longer prefix"
        };
        return Err(format!(
            "the prefix {target:?} matches {} staged sources: {matches}; {advice}",
            chosen.len()
        )
        .into());
    }
    if target != "all" && chosen.is_empty() {
        return Err(format!("no staged source matches the prefix {target:?}").into());
    }
    let mut lines = Vec::new();
    for (dir, meta) in &chosen {
        fs::remove_dir_all(dir).map_err(|error| format!("remove {}: {error}", dir.display()))?;
        lines.push(format!(
            "Removed: {} {} ({})",
            &meta.sha256[..12],
            meta.file_name,
            dir.display()
        ));
    }
    if lines.is_empty() {
        lines.push("Nothing to remove.".to_owned());
    }
    commands::result(lines.join("\n"), format)
}
