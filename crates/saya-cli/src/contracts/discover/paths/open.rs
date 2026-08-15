//! Safe open: open a candidate as a **regular file only**.
//!
//! Separate from [`super`]'s root resolution and [`super::entry`]'s containment
//! decision — this is the "never block on a FIFO/device, never open a
//! directory" concern, applied after a candidate has already passed the
//! inside-root check.

use std::fs;
use std::io;

use super::Candidate;

/// Open a candidate as a **regular file only**.
///
/// Order matters: a FIFO opened for reading with no writer blocks forever —
/// a DoS that looks like a hang (spec §4). So we check the file type with
/// `symlink_metadata` (no open, no block) **before** `File::open`, and reject
/// anything that is not a regular file without ever opening it. After opening
/// we re-verify the handle's own metadata (`fstat`) so a type-swap between the
/// pre-open check and the open is still caught for non-regular targets.
///
/// Race we do NOT close: a same-type regular-file swap between the pre-open
/// `symlink_metadata` and the open is not caught — both checks see a regular
/// file. Stated plainly in the module docs; the stdlib has no portable
/// `openat`-with-`O_NOFOLLOW` to do better.
///
/// Returns `Ok(None)` if the candidate is not a regular file (skip, do not
/// abort the pass); `Err` only on a read failure of the root itself.
pub(crate) fn open_regular(candidate: &Candidate) -> io::Result<Option<(fs::File, u64)>> {
    // Pre-open type check — never open a FIFO/device/directory. The path is
    // already canonical, so symlink_metadata reports the real target's type.
    let pre = fs::symlink_metadata(&candidate.canonical)?;
    if !pre.is_file() {
        return Ok(None);
    }
    let file = fs::File::open(&candidate.canonical)?;
    // Post-open re-verify via the handle (fstat), catching a swap to a
    // non-regular file between the two checks.
    let post = file.metadata()?;
    if !post.is_file() {
        return Ok(None);
    }
    Ok(Some((file, post.len())))
}
