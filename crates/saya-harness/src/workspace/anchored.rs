//! The contained file operations, served through the anchored walk (unix):
//! reads, writes, and listings. Each operation validates its argument once
//! through [`Workspace::anchor`] and then acts on the descriptor the walk
//! held — the pre-open scan is `fstatat` on the anchored parent, the open is
//! no-follow relative to it, and every identity check compares the scan
//! against what was actually opened. Nothing re-resolves a cumulative path,
//! so a swap of an intermediate directory between the walk and the open
//! cannot split the two.

use std::{ffi::OsStr, io, io::Read, os::fd::AsRawFd};

use sha2::{Digest, Sha256};

use super::anchor::Anchor;
use super::contain::{EntryKind, ListEntry, ReadFile, Workspace};
use super::fd::{DirStream, FinalStat, open_dir_at, stat_at};
use crate::{HarnessError, io_error};

/// Reads a workspace file under containment: the anchored walk, a no-follow
/// open, and a post-open identity check so the bytes served belong to the
/// file that was scanned. At most `max_bytes` are returned, with `truncated`
/// set when the file holds more. The digest covers the whole file — hashed
/// in the same open, streamed in 64 KiB chunks — so a truncated read still
/// names the state an edit precondition can state. Files larger than the
/// operation's file ceiling refuse before any byte is hashed: the refusal
/// names the scanned size and the bound, so a very large file costs a stat,
/// not a full hash. Ordinary reads retain the 8 MiB backstop; scratch CSV
/// import calls this same contained open with its separate 32 MiB ceiling.
pub(crate) fn read(
    ws: &Workspace,
    rel: &str,
    max_bytes: u64,
    file_limit: u64,
) -> Result<ReadFile, HarnessError> {
    let anchor = ws.anchor(rel, false)?;
    let stat = final_stat(&anchor, "read workspace file", rel)?;
    if stat.is_symlink() {
        return Err(HarnessError::SymlinkRefused {
            path: rel.to_string(),
        });
    }
    if !stat.is_file() {
        return Err(HarnessError::NotRegularFile {
            path: rel.to_string(),
        });
    }
    if stat.len() > file_limit {
        return Err(HarnessError::BoundsExceeded {
            path: rel.to_string(),
            found: stat.len(),
            max: file_limit,
        });
    }
    let file = anchor.open_verified(stat.identity(), rel)?;
    let mut bytes = Vec::new();
    let mut hasher = Sha256::new();
    let mut chunk = [0u8; 64 * 1024];
    let mut kept = 0u64;
    let mut scanned = 0u64;
    loop {
        let read = (&file)
            .read(&mut chunk)
            .map_err(|error| io_error("read workspace file", anchor.path(), error))?;
        if read == 0 {
            break;
        }
        scanned += read as u64;
        if scanned > file_limit {
            return Err(HarnessError::BoundsExceeded {
                path: rel.to_string(),
                found: scanned,
                max: file_limit,
            });
        }
        hasher.update(&chunk[..read]);
        let room = max_bytes.saturating_sub(kept) as usize;
        let take = read.min(room);
        bytes.extend_from_slice(&chunk[..take]);
        kept += take as u64;
    }
    let truncated = stat.len() > max_bytes;
    Ok(ReadFile {
        bytes,
        size: stat.len(),
        truncated,
        digest: hex_digest(&hasher.finalize()),
    })
}

/// Lowercase hex of a sha256 digest.
fn hex_digest(digest: &[u8]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Writes a workspace file atomically: temp file at 0600 inside the anchored
/// parent, fsync, rename over the target — all anchored to the parent the
/// walk validated, so the rename replaces whatever the path names *there*,
/// never wherever the path resolves now. The rename replaces without
/// following, so content cannot land outside the root; the destination is
/// re-verified afterwards so a post-write swap is reported rather than
/// pretended away. No execute bits are ever set.
pub(crate) fn write(ws: &Workspace, rel: &str, bytes: &[u8]) -> Result<(), HarnessError> {
    let anchor = ws.anchor(rel, true)?;
    if let Some(stat) = anchor.stat() {
        if stat.is_symlink() {
            return Err(HarnessError::SymlinkRefused {
                path: rel.to_string(),
            });
        }
        if !stat.is_file() {
            return Err(HarnessError::NotRegularFile {
                path: rel.to_string(),
            });
        }
    }
    anchor.commit_bytes(rel, bytes)
}

/// Lists a workspace directory, bounded by `max_entries`. The empty argument
/// names the workspace root itself. The directory is opened no-follow
/// relative to the anchored parent, so what is enumerated is the directory
/// the walk resolved.
pub(crate) fn list(
    ws: &Workspace,
    rel: &str,
    max_entries: usize,
) -> Result<Vec<ListEntry>, HarnessError> {
    // The listed directory is opened no-follow relative to the anchored
    // parent, and every entry's kind/size stat resolves against the stream's
    // own descriptor — never against a path that could be swapped underneath.
    let (dir_display, mut stream) = if rel.is_empty() {
        let root_fd = ws.root_dir_fd();
        (
            ws.root().to_path_buf(),
            DirStream::open_at(root_fd.as_raw_fd(), OsStr::new("."))
                .map_err(|error| io_error("list workspace directory", ws.root(), error))?,
        )
    } else {
        let anchor = ws.anchor(rel, false)?;
        let stat = final_stat(&anchor, "list workspace directory", rel)?;
        if !stat.is_dir() {
            return Err(HarnessError::NotRegularFile {
                path: rel.to_string(),
            });
        }
        let held = open_dir_at(anchor.dir_fd(), anchor.name()).map_err(|error| {
            if error.raw_os_error() == Some(libc::ELOOP) {
                return HarnessError::SymlinkRefused {
                    path: rel.to_string(),
                };
            }
            io_error("list workspace directory", anchor.path(), error)
        })?;
        let display = anchor.path().to_path_buf();
        // The held descriptor IS the directory to enumerate.
        let stream = DirStream::open_at(held.as_raw_fd(), OsStr::new("."))
            .map_err(|error| io_error("list workspace directory", &display, error))?;
        (display, stream)
    };
    let mut entries = Vec::new();
    while let Some((name, d_type)) = stream
        .next()
        .map_err(|error| io_error("list workspace directory", &dir_display, error))?
    {
        let name = name.into_string().map_err(|_| HarnessError::Io {
            context: format!(
                "workspace entry name is not UTF-8 in {}",
                dir_display.display()
            ),
            source: io::Error::new(io::ErrorKind::InvalidData, "entry name"),
        })?;
        // Kind from the directory entry when the filesystem reports it;
        // otherwise a no-follow stat against the stream's descriptor. The
        // size is that same no-follow stat, defaulting to zero as the
        // path-based listing did when the entry has already raced away.
        let stat = stat_at(stream.dir_fd(), OsStr::new(&name)).ok();
        let kind = match d_type {
            libc::DT_LNK => EntryKind::Symlink,
            libc::DT_DIR => EntryKind::Dir,
            libc::DT_REG => EntryKind::File,
            libc::DT_UNKNOWN => match stat.as_ref() {
                Some(stat) if stat.is_symlink() => EntryKind::Symlink,
                Some(stat) if stat.is_dir() => EntryKind::Dir,
                Some(stat) if stat.is_file() => EntryKind::File,
                _ => EntryKind::Other,
            },
            _ => EntryKind::Other,
        };
        entries.push(ListEntry {
            name,
            kind,
            size: stat.as_ref().map(FinalStat::len).unwrap_or(0),
        });
        // Checked after the push, so the refusal fires when a directory holds
        // MORE than `max_entries` entries — the (max+1)-th entry trips the
        // bound with found > max, matching the search bounds and the walk's
        // visited bound; a quietly shortened listing would read as the whole
        // directory.
        if entries.len() > max_entries {
            return Err(HarnessError::BoundsExceeded {
                path: rel.to_string(),
                found: entries.len() as u64,
                max: max_entries as u64,
            });
        }
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

pub(crate) fn final_stat<'a>(
    anchor: &'a Anchor,
    _context: &'static str,
    rel: &str,
) -> Result<&'a FinalStat, HarnessError> {
    anchor.stat().ok_or_else(|| HarnessError::NotFound {
        path: rel.to_string(),
    })
}
