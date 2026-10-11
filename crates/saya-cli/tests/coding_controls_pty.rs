#![cfg(not(windows))]
//! Real PTY acceptance source for selected live coding controls.
use std::time::Duration;

pub(crate) const DEADLINE: Duration = Duration::from_secs(20);
pub(crate) const INTERACTIVE: Duration = Duration::from_secs(15);
pub(crate) const OUTPUT_LIMIT: usize = 256 * 1024;

#[path = "coding_controls_pty/case.rs"]
mod case;
#[path = "coding_controls_pty/provider.rs"]
mod provider;
#[path = "coding_controls_pty/session.rs"]
mod session;

pub(crate) use provider::{Provider, config};
pub(crate) use session::Session;
