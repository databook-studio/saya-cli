//! Manifests for episode briefs: names, sizes, digests — never bulk
//! contents. The walk is deterministic (sorted) and refuses when the
//! file-count or per-file bound would be exceeded.

use std::{fs, io::Read, path::Path};

use sha2::{Digest, Sha256};

use crate::workspace::{MAX_IO_BYTES, MAX_LIST_ENTRIES, Workspace};
use crate::{HarnessError, io_error};

/// One workspace artifact as an episode brief names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestEntry {
    /// Slash-separated path, relative to the workspace root.
    pub path: String,
    pub size: u64,
    /// Lowercase hex sha256 of the file's bytes.
    pub digest: String,
}

/// Walks the workspace and digests every regular file. Hygiene content
/// (`.git`) and symlinks never enter the brief; every digest is read through
/// the same no-follow, identity-checked open a contained read uses.
pub fn build(
    ws: &Workspace,
    max_files: usize,
    max_file_bytes: u64,
) -> Result<Vec<ManifestEntry>, HarnessError> {
    if max_files > MAX_LIST_ENTRIES {
        return Err(HarnessError::BoundsExceeded {
            path: String::new(),
            found: max_files as u64,
            max: MAX_LIST_ENTRIES as u64,
        });
    }
    if max_file_bytes > MAX_IO_BYTES as u64 {
        return Err(HarnessError::BoundsExceeded {
            path: String::new(),
            found: max_file_bytes,
            max: MAX_IO_BYTES as u64,
        });
    }
    let mut state = Walk {
        ws,
        max_files,
        max_file_bytes,
        files: 0,
        entries: Vec::new(),
    };
    walk(ws.root(), "", &mut state)?;
    state.entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(state.entries)
}

/// Resolves one named workspace file to a brief-shaped manifest entry —
/// the same scan-then-no-follow-open-then-digest discipline the walk uses,
/// for one file. A missing file and a refused name (a link, an escape, an
/// over-bound file) are harness errors for the caller to classify; the
/// entry carries the name as given, plus size and digest.
pub fn entry(
    ws: &Workspace,
    rel: &str,
    max_file_bytes: u64,
) -> Result<ManifestEntry, HarnessError> {
    if max_file_bytes > MAX_IO_BYTES as u64 {
        return Err(HarnessError::BoundsExceeded {
            path: rel.to_string(),
            found: max_file_bytes,
            max: MAX_IO_BYTES as u64,
        });
    }
    let path = ws.target(rel, false)?;
    let pre = fs::symlink_metadata(&path)
        .map_err(|error| io_error("scan workspace file", &path, error))?;
    if pre.file_type().is_symlink() {
        return Err(HarnessError::SymlinkRefused {
            path: rel.to_string(),
        });
    }
    if !pre.is_file() {
        return Err(HarnessError::NotRegularFile {
            path: rel.to_string(),
        });
    }
    if pre.len() > max_file_bytes {
        return Err(HarnessError::BoundsExceeded {
            path: rel.to_string(),
            found: pre.len(),
            max: max_file_bytes,
        });
    }
    let digest = digest_file(ws, &path, &pre, rel, max_file_bytes)?;
    Ok(ManifestEntry {
        path: rel.to_string(),
        size: pre.len(),
        digest,
    })
}

struct Walk<'a> {
    ws: &'a Workspace,
    max_files: usize,
    max_file_bytes: u64,
    files: usize,
    entries: Vec<ManifestEntry>,
}

fn walk(dir: &Path, prefix: &str, state: &mut Walk) -> Result<(), HarnessError> {
    let mut names: Vec<String> = Vec::new();
    let read =
        fs::read_dir(dir).map_err(|error| io_error("scan workspace directory", dir, error))?;
    for entry in read {
        let entry = entry.map_err(|error| io_error("scan workspace directory", dir, error))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| HarnessError::Io {
                context: format!("workspace entry name is not UTF-8 in {}", dir.display()),
                source: std::io::Error::new(std::io::ErrorKind::InvalidData, "entry name"),
            })?;
        names.push(name);
    }
    names.sort();
    for name in names {
        let path = dir.join(&name);
        let meta = fs::symlink_metadata(&path)
            .map_err(|error| io_error("scan workspace directory", &path, error))?;
        if meta.file_type().is_symlink() || name.eq_ignore_ascii_case(".git") {
            continue;
        }
        if meta.is_dir() {
            let child = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            walk(&path, &child, state)?;
        } else if meta.is_file() {
            state.files += 1;
            if state.files > state.max_files {
                return Err(HarnessError::BoundsExceeded {
                    path: rel_path(prefix, &name),
                    found: state.files as u64,
                    max: state.max_files as u64,
                });
            }
            if meta.len() > state.max_file_bytes {
                return Err(HarnessError::BoundsExceeded {
                    path: rel_path(prefix, &name),
                    found: meta.len(),
                    max: state.max_file_bytes,
                });
            }
            let rel = rel_path(prefix, &name);
            let digest = digest_file(state.ws, &path, &meta, &rel, state.max_file_bytes)?;
            state.entries.push(ManifestEntry {
                path: rel,
                size: meta.len(),
                digest,
            });
        }
    }
    Ok(())
}

fn rel_path(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

fn digest_file(
    ws: &Workspace,
    path: &Path,
    pre: &fs::Metadata,
    rel: &str,
    bound: u64,
) -> Result<String, HarnessError> {
    let file = ws.open_verified(path, pre, rel)?;
    let mut hasher = Sha256::new();
    let mut taken = file.take(bound);
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let read = taken
            .read(&mut chunk)
            .map_err(|error| io_error("digest workspace file", path, error))?;
        if read == 0 {
            break;
        }
        hasher.update(&chunk[..read]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
