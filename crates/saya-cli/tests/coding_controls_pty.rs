#![cfg(not(windows))]
//! Real PTY acceptance source for selected live coding controls.
use std::time::Duration;

// Every wait below is a load ceiling, not a product bound: the live child, the
// PTY and the scripted provider all do their work on their own, and the same
// run takes several times longer on a machine that is also running the rest of
// the suite. A wait under a ceiling still ends the moment its condition holds.
/// The whole test's ceiling. It has to outlast the pause acknowledgement below,
/// or a slow run would fail on a provider that stopped serving instead.
pub(crate) const DEADLINE: Duration = Duration::from_secs(480);
/// A plain interactive round trip: a typed command, a rendered receipt.
pub(crate) const INTERACTIVE: Duration = Duration::from_secs(60);
/// The pause acknowledgement is delivered by the live child itself at its next
/// turn boundary, so its wait is longer than a plain receipt wait.
pub(crate) const PAUSE_ACK: Duration = Duration::from_secs(180);
pub(crate) const OUTPUT_LIMIT: usize = 256 * 1024;

#[path = "coding_controls_pty/case.rs"]
mod case;
#[path = "coding_controls_pty/provider.rs"]
mod provider;
#[path = "coding_controls_pty/session.rs"]
mod session;

pub(crate) use provider::{Provider, config};
pub(crate) use session::Session;
