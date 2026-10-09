//! Verbose-session metrics, printed only after the terminal is restored
//! (packet PH-G1): a real-PTY ordering proof.
//!
//! The test spawns the real `saya --verbose` TUI on a pty, waits for the
//! first paint, answers the startup trust modal, quits with Ctrl+C twice,
//! and reads every raw byte to EOF. The assertions are positional on the
//! raw stream: nothing metric-related may print while the alternate screen
//! is up, and the session metrics section, carrying exactly one first_paint
//! sample line, must land after the alternate-screen leave.
#![cfg(not(windows))]

/// Alternate-screen enter, byte-exact; never typed as an escape sequence.
const ALT_ENTER: [u8; 8] = [0x1b, b'[', b'?', b'1', b'0', b'4', b'9', b'h'];
/// Alternate-screen leave, byte-exact.
const ALT_LEAVE: [u8; 8] = [0x1b, b'[', b'?', b'1', b'0', b'4', b'9', b'l'];
/// The fixed metrics-section header needle.
const HEADER_TEXT: &[u8] = b"saya local metrics";
/// The first_paint sample line the restored section must carry, exactly once.
const FIRST_PAINT_LINE: &[u8] = b"phase first_paint samples=1";
/// How long any single interaction may take to paint.
const INTERACTIVE_WAIT: Duration = Duration::from_secs(15);
include!("tui_verbose_metrics_pty/support/bytes.rs");
include!("tui_verbose_metrics_pty/support/pty.rs");
include!("tui_verbose_metrics_pty/support/session.rs");
include!("tui_verbose_metrics_pty/cases/metrics.rs");
