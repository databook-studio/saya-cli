//! Bounded reopening of one discovered skill body.
//!
//! [`load_skill_body`] reopens the entry's `SKILL.md` through the header
//! reader's anchored no-follow seam, re-verifies the frontmatter identity,
//! and returns the body text plus the digest of the exact snapshot used.
//! The body is inert context that grants no authority.

use std::fmt;
#[cfg(unix)]
use std::io::Read;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

#[cfg(unix)]
use sha2::{Digest, Sha256};

#[cfg(unix)]
use crate::HarnessError;

#[cfg(unix)]
use super::{MAX_SKILL_HEADER_BYTES, frontmatter::parse_frontmatter};
use super::{SkillOrigin, SkillRoot, load_error::SkillLoadError, metadata::SkillEntry};

/// Maximum size of one skill body: 65,536 bytes; exactly the cap is
/// admitted, one byte over is refused. The combined read bound for the
/// whole file (header cap plus body cap) is a separate, larger limit.
pub const MAX_SKILL_BODY_BYTES: usize = 64 * 1024;

/// One reopened skill body: descriptive context only. The text never grants
/// authority — no accessor returns a permission, command, path, or tool.
/// [`Debug`] intentionally omits the text and every path.
#[derive(Clone, PartialEq, Eq)]
pub struct SkillBody {
    name: String,
    origin: SkillOrigin,
    trust_label: String,
    text: String,
    body_len: usize,
    source_sha256: String,
}

impl SkillBody {
    /// The validated frontmatter name recorded at discovery.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The descriptive origin recorded at discovery.
    pub fn origin(&self) -> SkillOrigin {
        self.origin
    }

    /// The caller-provided display label recorded at discovery.
    pub fn trust_label(&self) -> &str {
        &self.trust_label
    }

    /// The body text: inert context, never executed.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The body length in bytes.
    pub fn body_len(&self) -> usize {
        self.body_len
    }

    /// Lowercase SHA-256 of the whole file snapshot this loader read — the
    /// exact bytes this result is accountable for, header and body together.
    pub fn source_sha256(&self) -> &str {
        &self.source_sha256
    }
}

impl fmt::Debug for SkillBody {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SkillBody")
            .field("name", &self.name)
            .field("origin", &self.origin)
            .field("body_len", &self.body_len)
            .finish_non_exhaustive()
    }
}

/// Reopens and snapshots one discovered skill. `entry` must come from
/// `discover_skills` over the same `roots`; the reopened frontmatter is
/// re-verified against the entry's recorded metadata digest first.
pub fn load_skill_body(
    roots: &[SkillRoot],
    entry: &SkillEntry,
) -> Result<SkillBody, SkillLoadError> {
    if entry.root_index >= roots.len() {
        return Err(SkillLoadError::UnknownRoot);
    }
    #[cfg(not(unix))]
    return Err(SkillLoadError::Unsupported);
    #[cfg(unix)]
    load_unix(&roots[entry.root_index], entry)
}

#[cfg(unix)]
fn load_unix(root: &SkillRoot, entry: &SkillEntry) -> Result<SkillBody, SkillLoadError> {
    let relative_path = format!("{}/SKILL.md", entry.relative_dir);
    let anchor = root
        .workspace
        .anchor(&relative_path, false)
        .map_err(map_harness_error)?;
    let Some(stat) = anchor.stat() else {
        return Err(SkillLoadError::Missing);
    };
    if stat.is_symlink() {
        return Err(SkillLoadError::Symlink);
    }
    if !stat.is_file() {
        return Err(SkillLoadError::NotRegular);
    }
    // The header reader's seam: nonblocking no-follow open with post-open
    // type and identity checks, so a scan-to-open swap is rejected first.
    let file = anchor
        .open_file(libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOCTTY, 0)
        .map_err(map_open_error)?;
    let opened = file.metadata().map_err(|_| SkillLoadError::Unreadable)?;
    if !opened.is_file() {
        return Err(SkillLoadError::NotRegular);
    }
    if (opened.dev(), opened.ino()) != stat.identity() {
        return Err(SkillLoadError::MetadataChanged);
    }
    let snapshot = read_snapshot(file)?;
    let (header, body) = split_frontmatter(&snapshot).ok_or(SkillLoadError::MetadataChanged)?;
    if body.len() > MAX_SKILL_BODY_BYTES {
        return Err(SkillLoadError::TooLarge {
            max: MAX_SKILL_BODY_BYTES,
        });
    }
    verify_metadata(header, entry)?;
    let text = decode_body(body)?;
    Ok(SkillBody {
        name: entry.name.clone(),
        origin: entry.origin,
        trust_label: entry.trust_label.clone(),
        body_len: text.len(),
        text,
        source_sha256: digest_hex(&snapshot),
    })
}

/// Reads the whole file from the one verified handle, at most one byte past the combined cap.
#[cfg(unix)]
fn read_snapshot(file: std::fs::File) -> Result<Vec<u8>, SkillLoadError> {
    let cap = MAX_SKILL_HEADER_BYTES
        .checked_add(MAX_SKILL_BODY_BYTES)
        .ok_or(SkillLoadError::TooLarge {
            max: MAX_SKILL_BODY_BYTES,
        })?;
    let mut snapshot = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let Some(remaining) = cap
            .checked_add(1)
            .and_then(|limit| limit.checked_sub(snapshot.len()))
            .filter(|remaining| *remaining > 0)
        else {
            return Err(SkillLoadError::TooLarge { max: cap });
        };
        let read = (&file)
            .read(&mut chunk[..remaining.min(chunk.len())])
            .map_err(|_| SkillLoadError::Unreadable)?;
        if read == 0 {
            break;
        }
        snapshot.extend_from_slice(&chunk[..read]);
    }
    if snapshot.len() > cap {
        return Err(SkillLoadError::TooLarge { max: cap });
    }
    Ok(snapshot)
}

/// Splits the snapshot into raw frontmatter bytes (both delimiter lines
/// included) and the bytes after the closing delimiter line, with the same
/// line rules as the header reader. `None` when no newline-terminated
/// closing delimiter exists.
#[cfg(unix)]
fn split_frontmatter(snapshot: &[u8]) -> Option<(&[u8], &[u8])> {
    let mut line_start = 0;
    let mut line_number = 0;
    for (index, byte) in snapshot.iter().enumerate() {
        if *byte != b'\n' {
            continue;
        }
        let line = &snapshot[line_start..index];
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line_number > 0 && line == b"---" {
            return Some((&snapshot[..index + 1], &snapshot[index + 1..]));
        }
        line_number += 1;
        line_start = index + 1;
    }
    None
}

/// Confirms the reopened frontmatter still hashes to discovery's digest,
/// the parsed name still matches, and parse failures mean bytes changed.
#[cfg(unix)]
fn verify_metadata(header: &[u8], entry: &SkillEntry) -> Result<(), SkillLoadError> {
    let parsed = parse_frontmatter(header).map_err(|_| SkillLoadError::MetadataChanged)?;
    if parsed.name != entry.name || digest_hex(header) != entry.metadata_sha256 {
        return Err(SkillLoadError::MetadataChanged);
    }
    Ok(())
}

/// Decodes the body bytes as bounded text: UTF-8 without NUL.
#[cfg(unix)]
fn decode_body(body: &[u8]) -> Result<String, SkillLoadError> {
    let text = std::str::from_utf8(body).map_err(|_| SkillLoadError::InvalidText)?;
    if text.contains('\0') {
        return Err(SkillLoadError::InvalidText);
    }
    Ok(text.to_owned())
}

/// Lowercase hex SHA-256, matching the discovery record's format.
#[cfg(unix)]
fn digest_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(unix)]
fn map_harness_error(error: HarnessError) -> SkillLoadError {
    match error {
        HarnessError::NotFound { .. } => SkillLoadError::Missing,
        HarnessError::SymlinkRefused { .. } => SkillLoadError::Symlink,
        HarnessError::NotRegularFile { .. } => SkillLoadError::NotRegular,
        _ => SkillLoadError::Unreadable,
    }
}

#[cfg(unix)]
fn map_open_error(error: std::io::Error) -> SkillLoadError {
    if error.raw_os_error() == Some(libc::ELOOP) {
        SkillLoadError::Symlink
    } else if error.kind() == std::io::ErrorKind::NotFound {
        SkillLoadError::Missing
    } else {
        SkillLoadError::Unreadable
    }
}
