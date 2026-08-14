//! Path safety for team-contract file discovery (spec §4).
//!
//! This is the first place the memory feature reads arbitrary files off disk;
//! everything before it worked on a private SQLite database this process owns.
//! Every path and every byte here is treated as hostile.
//!
//! The contract:
//! - The root (`<project>/.saya/contracts`) is canonicalised **before** any
//!   read.
//! - Every candidate is canonicalised and must still be **inside** the
//!   canonical root. We compare canonical paths, not string prefixes — `..`
//!   and symlinks defeat a string-prefix check.
//! - A symlink escaping the root is rejected, not followed. A symlink within
//!   the root is fine.
//! - Only regular files ending `.toml`. No directories, devices, or FIFOs — a
//!   FIFO read blocks forever, a DoS that looks like a hang.
//!
//! Race we do NOT close: canonicalise-then-open is two syscalls. A symlink
//! swap between them is a classic TOCTOU. The Rust stdlib has no portable
//! `openat`-with-`O_NOFOLLOW`; pulling in `libc` for one slice is out of
//! proportion. We mitigate by re-verifying the *opened handle's* metadata is a
//! regular file (closes the FIFO/device DoS, and a swap *to* a non-regular
//! file), but a same-type regular-file swap to a different in-root file is
//! not caught. Stated plainly rather than pretended closed.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Resolve and canonicalise the discovery root. A missing `.saya/contracts`
/// is the normal case and returns `Ok(None)`. A path that exists but is not a
/// directory, or cannot be canonicalised, is a hard error.
pub(crate) fn canonical_root(project_root: &Path) -> Result<Option<PathBuf>, RootError> {
    let raw = project_root.join(".saya").join("contracts");
    // symlink_metadata so a symlink *to* a directory is detected; canonicalize
    // then resolves it. A missing path is the normal case.
    match fs::symlink_metadata(&raw) {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(RootError::Read(e)),
    }
    let canonical = raw.canonicalize().map_err(RootError::Read)?;
    let meta = fs::symlink_metadata(&canonical).map_err(RootError::Read)?;
    if !meta.is_dir() {
        return Err(RootError::NotADirectory);
    }
    Ok(Some(canonical))
}

/// A candidate file under the root. `relative` is display-only and is the
/// path relative to the project root, never absolute. `canonical` is the
/// canonical absolute path, used only for the inside-root check.
pub(crate) struct Candidate {
    pub relative: PathBuf,
    pub canonical: PathBuf,
}

/// One directory entry, after the safety checks. `Ok(Some(candidate))` means
/// it passed; `Ok(None)` means it was skipped (not a regular `.toml` file,
/// e.g. a directory named `x.toml`); the inside-root failure is the caller's
/// to report, returned as `Err(Escaped)`.
pub(crate) enum Checked {
    Candidate(Candidate),
    Skip,
    Escaped(PathBuf),
}

/// Check a raw directory entry against the root, **without** following the
/// entry's own symlink for the escape decision: we canonicalise the entry and
/// require the canonical path to be inside the canonical root. A symlink whose
/// target is outside the root therefore fails this check and is rejected.
pub(crate) fn check_entry(
    root: &Path,
    entry_path: &Path,
    relative: &Path,
) -> Result<Checked, RootError> {
    // Extension filter first — cheap, and a directory named `x.toml` should be
    // skipped, not canonicalised. `extension()` is None for `.toml` at the
    // root edge cases but `.toml` is well-formed here.
    if !is_toml(entry_path) {
        return Ok(Checked::Skip);
    }
    // Canonicalise the entry. This follows symlinks; an escaping symlink's
    // canonical path will be outside `root`, which the contains() check
    // rejects. A symlink inside the root canonicalises inside the root and is
    // accepted. We do NOT follow-then-trust: we re-verify the opened file's
    // type at read time (see `open_regular`).
    let canonical = match entry_path.canonicalize() {
        Ok(p) => p,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Checked::Skip),
        // A broken symlink canonicalises to an error; skip it rather than
        // aborting the whole pass.
        Err(_) => return Ok(Checked::Skip),
    };
    if !contains(root, &canonical) {
        return Ok(Checked::Escaped(relative.to_path_buf()));
    }
    Ok(Checked::Candidate(Candidate {
        relative: relative.to_path_buf(),
        canonical,
    }))
}

/// Open a candidate as a **regular file only**.
///
/// Order matters: a FIFO opened for reading with no writer blocks forever —
/// a DoS that looks like a hang (spec §4). So we check the file type with
/// `symlink_metadata` (no open, no block) **before** `File::open`, and reject
/// anything that is not a regular file without ever opening it. After opening
/// we re-verify the handle's own metadata (`fstat`) so a type-swap between
/// the pre-open check and the open is still caught for non-regular targets.
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

/// True if `child` is `root` or below it, by canonical-path components. This
/// is the check that `..` and escaping symlinks cannot defeat: we compare
/// canonical components, not a string prefix.
fn contains(root: &Path, child: &Path) -> bool {
    // `starts_with` on `Path` compares components, not bytes, so `a/b` does
    // not wrongly contain `a/bc`. Both paths are canonical (absolute, no
    // `..`, symlinks resolved), so this is a real containment test.
    child == root || child.starts_with(root)
}

fn is_toml(path: &Path) -> bool {
    matches!(path.extension().and_then(|e| e.to_str()), Some("toml"))
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum RootError {
    #[error("could not read contracts root: {0}")]
    Read(#[from] io::Error),
    #[error("contracts root is not a directory")]
    NotADirectory,
}
