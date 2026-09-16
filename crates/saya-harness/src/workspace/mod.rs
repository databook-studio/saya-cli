//! Contained access to a run's workspace directory — the only file I/O the
//! model-facing tools will ever get. The root is resolved once by the engine
//! and passed in; nothing here ever infers a path from the current directory.
//!
//! Mechanics, each one paired with an adversarial test in
//! `tests/workspace_containment.rs`:
//!
//! - **Argument validation** rejects NUL bytes, empty names, dot components,
//!   absolute forms, and Windows drive/UNC shapes before any filesystem call.
//! - **Symlinks are refused, not followed**, at every component, checked via
//!   `symlink_metadata` (which works on Windows too).
//! - `..` may cancel an already-validated component but never rises above
//!   the root; a canonicalise-then-prefix check confirms the resolved path
//!   stays under the canonical root.
//! - **The open itself is no-follow** (unix: `O_NOFOLLOW` via
//!   `OpenOptionsExt::custom_flags`) plus a post-open identity check — the
//!   (dev, inode) of the opened file must equal the pre-open scan — so a
//!   final-component swap between check and open cannot split the two.
//! - **Every component is anchored (unix)**: the walk resolves each
//!   component relative to a file descriptor for its parent (`openat`) and
//!   the root is pinned by a descriptor opened once, so the open, the temp
//!   creation, the rename, and the listing act on the directory the walk
//!   actually validated — a swap of an intermediate directory between the
//!   walk and the open cannot split the two. The walk lives in `dirfd`, with
//!   the descriptor mechanics in `fd`, the anchored handle in `anchor`, the
//!   file operations on anchors in `anchored`, and the download surface in
//!   `download`.
//! - **Writes are atomic** (temp + rename) and land at 0600 — no execute
//!   bits ever; the rename replaces without following, so content cannot
//!   land outside the root.
//! - `.git` names are denied as hygiene only — never a credentials control.
//!
//! Documented residual, until the runner's OS sandbox lands: a directory the
//! walk has anchored can still be renamed out of the tree by a writer that
//! can write to the tree, and the anchored open then acts on that directory
//! as validated — wherever it now lives. No fd-anchored design (including
//! Linux `openat2 RESOLVE_BENEATH`) can revoke a rename performed by the
//! hostile writer itself; every path-check's reach ends where the descriptor
//! begins. The manifest's own deterministic walk (`manifest::build`) still
//! resolves by path and shares this residual — engine-internal, bounded, and
//! closed by the sandbox. Stated here, not silent. On Windows the posture is
//! weaker still: there is no `openat` equivalent in the same shape, so no
//! anchored walk — containment there rests on the component-by-path walk,
//! the canonicalise-then-prefix check, the metadata checks, and a weaker
//! post-open identity tuple (length, creation time) — a stated, weaker
//! TOCTOU posture that includes the intermediate-component residual above;
//! the runner fails closed on Windows regardless.

#[cfg(unix)]
pub(crate) mod anchor;
#[cfg(unix)]
pub(crate) mod anchored;
pub mod contain;
#[cfg(unix)]
pub(crate) mod dirfd;
#[cfg(unix)]
pub(crate) mod download;
#[cfg(unix)]
pub(crate) mod fd;
pub mod manifest;
pub mod patch;
pub mod pattern;
pub mod search;
pub mod walk;

pub use contain::{EntryKind, ListEntry, ReadFile, Workspace};
pub use search::{GlobMatch, GrepMatch, GrepOutcome};
