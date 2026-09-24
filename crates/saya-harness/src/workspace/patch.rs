//! The anchored range-replace: swap one byte range of a contained file for
//! replacement bytes, committed atomically. The walk, the no-follow open,
//! and the temp+rename commit are the same operations the whole-file write
//! uses — this module only reads the old content, splices the range, and
//! hands the result to [`Anchor::commit_bytes`]. How the range was chosen
//! (matching text, counting matches) belongs to a later slice, not here.
//!
//! The positional precondition `expected_len` is what lets a later append
//! variant express "empty range at EOF" through this same path: the caller
//! states the size it measured, and a file that no longer has that size is
//! refused untouched rather than spliced against stale offsets.

#[cfg(unix)]
use std::io::Read as _;

use super::contain::Workspace;
use crate::HarnessError;

#[cfg(unix)]
use crate::io_error;

/// Replacement bytes per patch call: the same 64 KiB round-trip discipline
/// as the whole-file write, so anything a patch inserts can be read back
/// whole. Over the bound is a typed whole-refusal, never a truncation.
pub const PATCH_REPLACEMENT_MAX_BYTES: usize = 64 * 1024;

/// Patch targets are read whole to splice the range, so targets are capped
/// at 8 MiB with a typed refusal naming the size and the cap. Larger files
/// stay editable through the process lanes under their own gates.
pub const PATCH_MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

impl Workspace {
    /// Replaces `range` of the contained file `rel` with `replacement`,
    /// atomically: a crash before the rename leaves the old content intact,
    /// never a truncation, and every refusal leaves the file byte-identical.
    /// `expected_len` must equal the file's current size — the positional
    /// precondition a later append variant reuses for its offset check.
    /// The range may be empty (a pure insertion); it may sit at EOF.
    /// The target must already exist: patching never creates a file.
    pub fn patch_range(
        &self,
        rel: &str,
        range: std::ops::Range<u64>,
        expected_len: u64,
        replacement: &[u8],
    ) -> Result<(), HarnessError> {
        if replacement.len() > PATCH_REPLACEMENT_MAX_BYTES {
            return Err(HarnessError::BoundsExceeded {
                path: rel.to_string(),
                found: replacement.len() as u64,
                max: PATCH_REPLACEMENT_MAX_BYTES as u64,
            });
        }
        #[cfg(unix)]
        return self.patch_range_anchored(rel, range, expected_len, replacement);

        #[cfg(not(unix))]
        return self.patch_range_by_path(rel, range, expected_len, replacement);
    }

    /// Splices `replacement` into `current` at `range`, after the
    /// precondition and bound checks. Pure: every refusal path returns before
    /// any byte is written anywhere.
    pub(crate) fn splice(
        rel: &str,
        range: std::ops::Range<u64>,
        expected_len: u64,
        replacement: &[u8],
        current: &[u8],
    ) -> Result<Vec<u8>, HarnessError> {
        let size = current.len() as u64;
        if size != expected_len {
            return Err(HarnessError::LengthMismatch {
                path: rel.to_string(),
                expected: expected_len,
                current: size,
            });
        }
        if range.start > range.end || range.end > size {
            return Err(HarnessError::RangeOutOfBounds {
                path: rel.to_string(),
                start: range.start,
                end: range.end,
                size,
            });
        }
        let (start, end) = (range.start as usize, range.end as usize);
        let mut patched = Vec::with_capacity(current.len() + replacement.len());
        patched.extend_from_slice(&current[..start]);
        patched.extend_from_slice(replacement);
        patched.extend_from_slice(&current[end..]);
        Ok(patched)
    }

    #[cfg(unix)]
    fn patch_range_anchored(
        &self,
        rel: &str,
        range: std::ops::Range<u64>,
        expected_len: u64,
        replacement: &[u8],
    ) -> Result<(), HarnessError> {
        let anchor = self.anchor(rel, false)?;
        let stat = super::anchored::final_stat(&anchor, "patch workspace file", rel)?;
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
        if stat.len() > PATCH_MAX_FILE_BYTES {
            return Err(HarnessError::BoundsExceeded {
                path: rel.to_string(),
                found: stat.len(),
                max: PATCH_MAX_FILE_BYTES,
            });
        }
        let file = anchor.open_verified(stat.identity(), rel)?;
        let mut current = Vec::new();
        (&file)
            .take(PATCH_MAX_FILE_BYTES.saturating_add(1))
            .read_to_end(&mut current)
            .map_err(|error| io_error("read workspace file", anchor.path(), error))?;
        let patched = Self::splice(rel, range, expected_len, replacement, &current)?;
        anchor.commit_bytes(rel, &patched)
    }

    #[cfg(not(unix))]
    fn patch_range_by_path(
        &self,
        rel: &str,
        range: std::ops::Range<u64>,
        expected_len: u64,
        replacement: &[u8],
    ) -> Result<(), HarnessError> {
        use std::io::Read as _;

        let path = self.target(rel, false)?;
        let pre = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(HarnessError::NotFound {
                    path: rel.to_string(),
                });
            }
            Err(error) => return Err(crate::io_error("patch workspace file", &path, error)),
        };
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
        if pre.len() > PATCH_MAX_FILE_BYTES {
            return Err(HarnessError::BoundsExceeded {
                path: rel.to_string(),
                found: pre.len(),
                max: PATCH_MAX_FILE_BYTES,
            });
        }
        let file = self.open_verified(&path, &pre, rel)?;
        let mut current = Vec::new();
        (&file)
            .take(PATCH_MAX_FILE_BYTES.saturating_add(1))
            .read_to_end(&mut current)
            .map_err(|error| crate::io_error("read workspace file", &path, error))?;
        let patched = Self::splice(rel, range, expected_len, replacement, &current)?;
        let parent = path.parent().unwrap_or(self.root()).to_path_buf();
        let (temp_path, mut temp) = self.create_temp(&parent, rel)?;
        use std::io::Write as _;
        temp.write_all(&patched)
            .map_err(|error| crate::io_error("write workspace temp", &temp_path, error))?;
        temp.sync_all()
            .map_err(|error| crate::io_error("sync workspace temp", &temp_path, error))?;
        let written = super::contain::file_identity(&temp)
            .map_err(|error| crate::io_error("stat workspace temp", &temp_path, error))?;
        drop(temp);
        super::contain::replace_workspace_file(&temp_path, &path)?;
        let final_meta = std::fs::symlink_metadata(&path)
            .map_err(|error| crate::io_error("verify written workspace file", &path, error))?;
        let committed = super::contain::path_identity(&path)
            .map_err(|error| crate::io_error("verify written workspace file", &path, error))?;
        if committed != written || !final_meta.is_file() {
            return Err(HarnessError::IdentityChanged {
                path: rel.to_string(),
            });
        }
        Ok(())
    }
}
