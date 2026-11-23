#![cfg(not(windows))]

//! Real-terminal proof for the approval surface (UI3): the panel an operator
//! answers is what the terminal shows, at both review sizes.
//!
//! One fixture — a coding job's consent, the call this surface receives from a
//! `/start` — is laid out twice: 100x30 and 60x24. The assertions are the
//! panel's own words and the fact that they are inside a box; the capture each
//! case prints is the screen text a reviewer reads in `REPORT.md`.
//!
//! Every wait is a load ceiling, not a product bound.

use std::time::Duration;

/// The whole case's ceiling: the child starts, paints and is asked once.
pub(crate) const DEADLINE: Duration = Duration::from_secs(120);
/// A wait for one paint: short, because the surface repaints thirty times a
/// second while it has something to say.
pub(crate) const PAINT: Duration = Duration::from_secs(60);
const IO_WAIT: Duration = Duration::from_millis(100);
/// How long the screen must stay the same before a capture reads it: a frame
/// arrives in chunks, and half a panel is not what an operator sees.
pub(crate) const SETTLE: Duration = Duration::from_millis(600);
const OUTPUT_LIMIT: usize = 256 * 1024;

#[path = "approval_panel_pty/cases.rs"]
mod cases;
#[path = "approval_panel_pty/harness.rs"]
pub(crate) mod harness;
