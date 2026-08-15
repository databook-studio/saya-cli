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
//!
//! Three concerns, three submodules: [`entry`] decides which directory entries
//! are candidates under the root; [`open`] opens a candidate as a regular
//! file only; this module resolves the root and holds the shared [`contains`]
//! primitive both sides (and the export *write* side, `io::write`) reuse.

mod entry;
mod open;

pub(crate) use entry::check_entry;
pub(crate) use open::open_regular;

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

/// True if `child` is `root` or below it, by canonical-path components. This
/// is the check that `..` and escaping symlinks cannot defeat: we compare
/// canonical components, not a string prefix. Reused by the export *write*
/// side (slice 6b, see `contracts::io::write`) so reading and writing share
/// one containment check rather than two.
pub(crate) fn contains(root: &Path, child: &Path) -> bool {
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

#[cfg(test)]
mod property_tests {
    //! Property 6 (spec §2): discovery never escapes its root. The safety core
    //! is [`contains`] — the component-structural containment test that `..` and
    //! escaping symlinks cannot defeat *once both paths are canonical* (the real
    //! `check_entry` canonicalises the candidate before calling `contains`). This
    //! module property-tests [`contains`] directly over component sequences so it
    //! explores traversal shapes (`..`, `.`, empty, unicode) rather than random
    //! bytes, with no filesystem and no async.
    //!
    //! The two properties: (a) `contains` is a *component*-prefix test, not a
    //! byte-prefix test — so `/a/b` does not contain `/a/bc` — verified against an
    //! independent oracle built from `Path::components`; (b) on canonical-shape
    //! paths (no `..`, `.`, or empty — what `canonicalize` actually produces)
    //! `contains`-true implies genuine structural containment. A regression to a
    //! byte `str::starts_with` fails (a): the oracle is component-structural.
    use super::contains;
    use proptest::prelude::*;
    use std::path::{Component, Path, PathBuf};

    /// One path component. The vocabulary includes single letters (so `b` vs
    /// `bc` exercises the byte-prefix trap), `..`, `.`, empty, and unicode.
    fn component() -> impl Strategy<Value = String> {
        prop::sample::select(vec![
            "a".to_string(),
            "b".to_string(),
            "bc".to_string(),
            "c".to_string(),
            "..".to_string(),
            ".".to_string(),
            String::new(),
            "é".to_string(),
        ])
    }

    /// A canonical-shape component: a real name, no traversal tokens. This is
    /// what `Path::canonicalize` yields — no `..`, `.`, or empty.
    fn canonical_component() -> impl Strategy<Value = String> {
        prop::sample::select(vec![
            "a".to_string(),
            "b".to_string(),
            "bc".to_string(),
            "c".to_string(),
            "orders".to_string(),
            "é".to_string(),
        ])
    }

    fn components_strategy(
        strategy: impl Strategy<Value = String>,
    ) -> impl Strategy<Value = Vec<String>> {
        prop::collection::vec(strategy, 1..=5)
    }

    /// Build an absolute path from raw components by joining with `/`, preserving
    /// `..`, `.`, and empty as the string parser would, then let `PathBuf` parse
    /// it. This is how traversal shapes reach `contains` in practice.
    fn build(comps: &[String]) -> PathBuf {
        let mut s = String::from("/");
        s.push_str(&comps.join("/"));
        PathBuf::from(s)
    }

    /// Independent oracle: `child` is contained iff its `Path::components` list
    /// begins with `root`'s component list. Built from `components()`, so it is
    /// component-structural and disagrees with any byte-prefix implementation.
    fn oracle_comps(p: &Path) -> Vec<String> {
        p.components()
            .map(|c| match c {
                Component::RootDir => "/".to_string(),
                Component::Normal(s) => s.to_string_lossy().into_owned(),
                Component::ParentDir => "..".to_string(),
                Component::CurDir => ".".to_string(),
                Component::Prefix(p) => p.as_os_str().to_string_lossy().into_owned(),
            })
            .collect()
    }

    fn oracle(root: &Path, child: &Path) -> bool {
        let r = oracle_comps(root);
        let c = oracle_comps(child);
        c.len() >= r.len() && c[..r.len()] == r[..]
    }

    proptest! {
        /// Property 6a — `contains` equals the independent component-prefix
        /// oracle over traversal shapes (`..`, `.`, empty, unicode). A switch to
        /// a byte `str::starts_with` diverges here on `/a/b` vs `/a/bc`.
        #[test]
        fn contains_is_component_structural(
            root_comps in components_strategy(component()),
            child_comps in components_strategy(component()),
        ) {
            let root = build(&root_comps);
            let child = build(&child_comps);
            prop_assert_eq!(
                contains(&root, &child),
                oracle(&root, &child),
                "root={:?} child={:?}",
                root,
                child
            );
        }

        /// Property 6b — on canonical-shape paths (what `canonicalize` produces,
        /// no `..`/`.`/empty), `contains`-true implies genuine structural
        /// containment: the child's components extend the root's exactly.
        #[test]
        fn contains_sound_on_canonical_shapes(
            root_comps in components_strategy(canonical_component()),
            child_comps in components_strategy(canonical_component()),
        ) {
            let root = build(&root_comps);
            let child = build(&child_comps);
            if contains(&root, &child) {
                let r = oracle_comps(&root);
                let c = oracle_comps(&child);
                prop_assert!(
                    c.len() >= r.len() && c[..r.len()] == r[..],
                    "canonical child contained but not a component extension: root={:?} child={:?}",
                    root,
                    child
                );
            }
        }
    }
}
