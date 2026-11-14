//! The launch posture derivation (`session_loop.rs`): every launch kind —
//! fresh, `--continue`, `--resume` — derives the status header's facts from
//! the universe the launch composed, and the deny journal stays a fresh-start
//! write. A resumed launch's universe composes the host lane wherever a root
//! binds (the same composer behind both paths), so a derivation guarded to
//! fresh starts leaves the header on `host:off` while the lane executes.

use crate::interactive::session::turn_loop::derive_launch_posture;

#[cfg(test)]
#[path = "launch_posture_tests/continued.rs"]
mod continued;
#[cfg(test)]
#[path = "launch_posture_tests/fresh.rs"]
mod fresh;
#[cfg(test)]
#[path = "launch_posture_tests/resumed.rs"]
mod resumed;
#[cfg(test)]
#[path = "launch_posture_tests/support.rs"]
mod support;
