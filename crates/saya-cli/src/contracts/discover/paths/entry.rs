//! Per-entry containment: decide whether one directory entry is a candidate
//! under the canonical root, **without** following the entry's own symlink for
//! the escape decision. Separate from [`super`]'s root resolution and
//! [`super::open`]'s safe-open: this is the "which entries survive" concern.

use std::io;
use std::path::Path;

use super::{Candidate, Checked, RootError, contains, is_toml};

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
