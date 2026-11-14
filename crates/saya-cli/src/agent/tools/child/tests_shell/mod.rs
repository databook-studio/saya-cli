//! The helper's `shell` lane: the same tool the root advertises,
//! over the shared working checkout, with this helper's own scratch tree and a
//! subject line that names it. The toolset advertises `shell` only when the
//! composition root attached one; a host call follows the job's Ask/Bypass
//! rule; concurrent helper asks queue, one prompt at a time; and a read-only
//! (`explore`) helper's shell is sandbox-only, where nothing is writable.
//!
//! Authored, not executed where the host proves no sandbox: every case that
//! needs a proven sandbox returns early, the way the root shell's battery does.
#![cfg(any(target_os = "macos", target_os = "linux"))]

mod advertisement;
mod approval;
mod background;
mod read_only;
mod serialisation;
mod support;

/// The helper's shell, built exactly as `compose_child` builds it, is shared
/// with `tests_loop`; the rest of the fixture lives in [`support`].
pub(super) use support::helper_shell;
