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
//! - **Writes are atomic** (temp + rename) and land at 0600 — no execute
//!   bits ever; the rename replaces whatever the path names without
//!   following it, so content cannot land outside the root.
//! - `.git` names are denied as hygiene only — never a credentials control.
//!
//! Documented residual, until the runner's OS sandbox lands: the
//! check-then-open chain is not atomic for *intermediate* path components.
//! A swap of an intermediate directory between the walk and the open is out
//! of reach of these guards; the sole writer of a run directory is the
//! engine, and the M5 sandbox closes the route entirely. Stated here, not
//! silent. On Windows the posture is weaker still: `O_NOFOLLOW` is
//! unavailable, so containment there rests on the metadata checks plus a
//! weaker post-open identity tuple (length, creation time) — a stated,
//! weaker TOCTOU posture; the runner fails closed on Windows regardless.

pub mod contain;
pub mod manifest;
pub mod pattern;
pub mod search;
pub mod walk;

pub use contain::{EntryKind, ListEntry, ReadFile, Workspace};
pub use search::{GlobMatch, GrepMatch, GrepOutcome};
