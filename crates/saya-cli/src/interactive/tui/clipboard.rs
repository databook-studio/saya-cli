//! Clipboard helpers: native OS tools and OSC 52 for SSH sessions.

use std::io::{self, Write};

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
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        if let Ok(status) = child.wait()
            && status.success()
        {
            return true;
        }
    }
    false
}

/// Copies `text` to the terminal's clipboard using the OSC 52 escape sequence.
/// This delegates to the terminal emulator, so it needs no native clipboard
/// library and works across an SSH session. Terminals that don't support OSC 52
/// simply ignore it.
pub(crate) fn osc52_copy<W: Write>(writer: &mut W, text: &str) -> io::Result<()> {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    // OSC 52 form: ESC ] 52 ; c ; <base64> BEL — `c` targets the clipboard.
    write!(writer, "\x1b]52;c;{encoded}\x07")?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::{clipboard_commands, osc52_copy};

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
}
