//! Clipboard helpers: native OS tools and OSC 52 for SSH sessions.

use std::io::{self, Write};
use std::thread;
use std::time::{Duration, Instant};

const CLIPBOARD_COMMAND_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_OSC52_BYTES: usize = 100 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ClipboardOutcome {
    Native,
    Osc52,
    Failed,
}

pub(crate) fn clipboard_outcome(native_ok: bool, osc_error: Option<&str>) -> ClipboardOutcome {
    match (native_ok, osc_error) {
        (true, _) => ClipboardOutcome::Native,
        (false, None) => ClipboardOutcome::Osc52,
        (false, Some(_)) => ClipboardOutcome::Failed,
    }
}

/// The OS clipboard command(s) to try, best candidate first. Each entry is an
/// argv whose program reads the clipboard payload from stdin.
pub(crate) fn clipboard_commands() -> &'static [&'static [&'static str]] {
    #[cfg(target_os = "macos")]
    {
        &[&["pbcopy"]]
    }
    #[cfg(target_os = "windows")]
    {
        &[&["clip"]]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // Wayland first, then the two common X11 tools.
        &[
            &["wl-copy"],
            &["xclip", "-selection", "clipboard"],
            &["xsel", "--clipboard", "--input"],
        ]
    }
}

/// Copies `text` to the OS clipboard by piping it to the first available
/// platform clipboard tool. Returns `true` on success. This is what makes copy
/// work in terminals that ignore OSC 52 (e.g. macOS Terminal.app).
pub(crate) fn copy_to_native_clipboard(text: &str) -> bool {
    use std::process::{Command, Stdio};
    for argv in clipboard_commands() {
        let Some((program, args)) = argv.split_first() else {
            continue;
        };
        let child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut child) = child else { continue };
        // Write the payload, then drop stdin to signal EOF before waiting.
        if let Some(mut stdin) = child.stdin.take()
            && stdin.write_all(text.as_bytes()).is_err()
        {
            let _ = child.kill();
            let _ = child.wait();
            continue;
        }
        let deadline = Instant::now() + CLIPBOARD_COMMAND_TIMEOUT;
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return true,
                Ok(Some(_)) | Err(_) => break,
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                Ok(None) => thread::sleep(Duration::from_millis(10)),
            }
        }
    }
    false
}

/// Copies `text` to the terminal's clipboard using the OSC 52 escape sequence.
/// This delegates to the terminal emulator, so it needs no native clipboard
/// library and works across an SSH session. Terminals that don't support OSC 52
/// simply ignore it.
pub(crate) fn osc52_copy<W: Write>(writer: &mut W, text: &str) -> io::Result<()> {
    if text.len() > MAX_OSC52_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "clipboard is too large for OSC 52; use a local clipboard tool",
        ));
    }
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    // OSC 52 form: ESC ] 52; c; <base64> BEL — `c` targets the clipboard.
    write!(writer, "\x1b]52;c;{encoded}\x07")?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::{ClipboardOutcome, clipboard_commands, clipboard_outcome, osc52_copy};

    #[test]
    fn clipboard_commands_are_non_empty_argvs() {
        let commands = clipboard_commands();
        assert!(
            !commands.is_empty(),
            "every platform needs a clipboard tool"
        );
        assert!(
            commands.iter().all(|argv| !argv.is_empty()),
            "each argv must at least name a program"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_prefers_pbcopy() {
        assert_eq!(clipboard_commands()[0][0], "pbcopy");
    }

    #[test]
    fn osc52_wraps_base64_in_the_clipboard_escape() {
        let mut out = Vec::new();
        osc52_copy(&mut out, "SELECT 1").expect("write to a Vec never fails");
        // "SELECT 1" base64-encodes to "U0VMRUNUIDE=".
        assert_eq!(out, b"\x1b]52;c;U0VMRUNUIDE=\x07");
    }

    #[test]
    fn osc52_encodes_multibyte_and_newlines() {
        let mut out = Vec::new();
        osc52_copy(&mut out, "café\n").expect("write to a Vec never fails");
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("\x1b]52;c;") && text.ends_with('\x07'));
        // The payload is base64 (no raw newline leaks into the escape).
        assert!(!text.contains('\n'));
    }

    #[test]
    fn clipboard_outcome_reports_native_osc52_and_failure_paths() {
        assert_eq!(clipboard_outcome(true, None), ClipboardOutcome::Native);
        assert_eq!(clipboard_outcome(false, None), ClipboardOutcome::Osc52);
        assert_eq!(
            clipboard_outcome(false, Some("broken pipe")),
            ClipboardOutcome::Failed
        );
    }

    #[test]
    fn osc52_rejects_oversized_payloads() {
        let mut out = Vec::new();
        let payload = "x".repeat(super::MAX_OSC52_BYTES + 1);
        let error = osc52_copy(&mut out, &payload).expect_err("oversized OSC 52 must be rejected");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(out.is_empty());
    }
}
