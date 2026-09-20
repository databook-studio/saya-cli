//! Fulfilling queued clipboard copies without blocking the event loop.

use super::super::clipboard::{
    ClipboardOutcome, clipboard_outcome, copy_to_native_clipboard, osc52_copy,
};
use super::super::transcript::BlockKind;
use super::super::types::{App, ClipboardCopy};
use ratatui::backend::CrosstermBackend;
use std::io::Stdout;

// Fulfil queued clipboard copies without blocking the event loop. Try the OS clipboard tool first (pbcopy
// / wl-copy / xclip / clip) since that's what actually works locally —
// notably in macOS Terminal.app, which ignores OSC 52. Always also emit
// OSC 52 so copies can still reach the *local* clipboard over SSH. Report
// the outcome only after attempting both mechanisms.
pub(crate) fn tick_clipboard(app: &mut App, backend: &mut CrosstermBackend<Stdout>) {
    if app.clipboard_copy.is_none()
        && let Some(text) = app.pending_clipboard.take()
    {
        let (sender, receiver) = std::sync::mpsc::channel();
        let native_text = text.clone();
        std::thread::spawn(move || {
            let _ = sender.send(copy_to_native_clipboard(&native_text));
        });
        app.clipboard_copy = Some(ClipboardCopy {
            native_result: receiver,
            osc_error: osc52_copy(backend, &text)
                .err()
                .map(|error| error.to_string()),
        });
    }
    let native_result =
        app.clipboard_copy
            .as_ref()
            .and_then(|copy| match copy.native_result.try_recv() {
                Ok(result) => Some(result),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(false),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            });
    if let Some(native_ok) = native_result
        && let Some(copy) = app.clipboard_copy.take()
    {
        match clipboard_outcome(native_ok, copy.osc_error.as_deref()) {
            ClipboardOutcome::Native => app
                .transcript
                .push(BlockKind::System, "Copied to the system clipboard."),
            ClipboardOutcome::Osc52 => app.transcript.push(
                BlockKind::System,
                "Sent OSC 52 clipboard data; it will copy if your terminal supports it.",
            ),
            ClipboardOutcome::Failed => app.transcript.push(
                BlockKind::Error,
                format!(
                    "Could not copy to the system clipboard, and sending OSC 52 failed: {}",
                    copy.osc_error
                        .unwrap_or_else(|| "unknown terminal output error".into())
                ),
            ),
        }
    }
}
