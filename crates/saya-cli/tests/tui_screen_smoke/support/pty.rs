// The parsed screen and the waits a case runs against it.
//
// A pty read is blocking with no portable deadline, so a reader thread owns it
// (`support/spawn.rs`) and this type drains chunks it ships. Waits are bounded
// twice: by the caller's window and by a hard ceiling, so a wedged terminal
// fails with the screen and the raw bytes instead of hanging CI.

/// The terminal size every launch asks for: wide enough for the full composer
/// hint, tall enough for the welcome box and the composer together.
const COLS: u16 = 100;
const ROWS: u16 = 30;

/// How long one blocking read waits before the caller checks its own deadline.
const READ_TIMEOUT: Duration = Duration::from_millis(100);
/// How long one painted state may take to appear.
const INTERACTIVE: Duration = Duration::from_secs(15);
/// Hard ceiling on a whole test so a wedged terminal can never hang CI.
const HARD_DEADLINE: Duration = Duration::from_secs(30);
/// How many raw bytes a failure message quotes when the screen is unreadable.
const RAW_TAIL: usize = 2_000;

/// A live `saya` on a pty: the writer feeds keys, the reader thread ships
/// chunks, and the parser turns them into the screen text the case asserts.
struct Tui {
    writer: Box<dyn Write + Send>,
    parser: vt100::Parser,
    rx: std::sync::mpsc::Receiver<std::io::Result<Vec<u8>>>,
    /// The pty master, held for the session: on Windows it owns the ConPTY.
    _master: Box<dyn portable_pty::MasterPty + Send>,
    child: ChildGuard,
    raw: Vec<u8>,
    /// The terminal side: ConPTY holds the child until its startup queries
    /// are answered (`support/answers.rs`).
    answers: Answers,
}

impl Tui {
    fn send(&mut self, text: &str) -> std::io::Result<()> {
        self.writer.write_all(text.as_bytes())?;
        self.writer.flush()
    }

    /// Types one line and presses Enter. `\r` is what a terminal's Enter key
    /// sends, and what crossterm reads as `KeyCode::Enter` on every platform.
    fn send_line(&mut self, line: &str) -> std::io::Result<()> {
        self.send(&format!("{line}\r"))
    }

    fn screen(&self) -> String {
        screen_text(self.parser.screen())
    }

    /// Waits until `needle` is on screen. A pty EOF first means the surface
    /// died, so the error names that rather than reporting a bare timeout.
    fn wait_for(&mut self, needle: &str, within: Duration) -> Result<(), String> {
        let start = Instant::now();
        loop {
            if self.screen().contains(needle) {
                return Ok(());
            }
            if start.elapsed() >= within || start.elapsed() >= HARD_DEADLINE {
                return Err(format!(
                    "'{needle}' never appeared within {within:?}; screen:\n{}\nraw tail:\n{}",
                    self.screen(),
                    self.raw_tail()
                ));
            }
            self.pump()?;
        }
    }

    /// Waits until `needle` has left the screen: the proof a pane closed
    /// rather than that its successor painted over it.
    fn wait_gone(&mut self, needle: &str, within: Duration) -> Result<(), String> {
        let start = Instant::now();
        loop {
            if !self.screen().contains(needle) {
                return Ok(());
            }
            if start.elapsed() >= within || start.elapsed() >= HARD_DEADLINE {
                return Err(format!(
                    "'{needle}' never left within {within:?}; screen:\n{}\nraw tail:\n{}",
                    self.screen(),
                    self.raw_tail()
                ));
            }
            self.pump()?;
        }
    }

    /// Waits until the child has ended, draining its output as it goes: the
    /// proof the surface left on its own after `/exit`.
    fn wait_exit(&mut self, within: Duration) -> Result<(), String> {
        let start = Instant::now();
        loop {
            if matches!(self.child.child.try_wait(), Ok(Some(_))) {
                return Ok(());
            }
            if start.elapsed() >= within {
                return Err(format!(
                    "the surface never left within {within:?}; screen:\n{}\nraw tail:\n{}",
                    self.screen(),
                    self.raw_tail()
                ));
            }
            // An EOF here is the reader thread finishing as the child exits;
            // `try_wait` is the authority, so a read error is not fatal.
            let _ = self.pump();
        }
    }

    fn still_running(&mut self) -> bool {
        matches!(self.child.child.try_wait(), Ok(None))
    }

    /// Drains the reader channel once, bounded by [`READ_TIMEOUT`]. An empty
    /// read is EOF: the surface exited, which is a failure for a case that is
    /// still waiting for it to paint.
    fn pump(&mut self) -> Result<(), String> {
        match self.rx.recv_timeout(READ_TIMEOUT) {
            Ok(Ok(bytes)) if bytes.is_empty() => Err(format!(
                "the TUI exited; screen:\n{}\nraw tail:\n{}",
                self.screen(),
                self.raw_tail()
            )),
            Ok(Ok(bytes)) => {
                self.raw.extend_from_slice(&bytes);
                self.parser.process(&bytes);
                self.answer_queries(&bytes)?;
                Ok(())
            }
            Ok(Err(error)) => Err(format!("pty read failed: {error}")),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Ok(()),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                Err("pty reader ended".to_string())
            }
        }
    }

    /// Writes the replies a terminal would send for any query in `bytes`.
    /// ConPTY's startup `ESC [ 6 n` is the one that matters: it holds every
    /// byte of child output until answered. The cursor reported is the
    /// parser's own, 1-based, so it is `1;1` before anything paints.
    fn answer_queries(&mut self, bytes: &[u8]) -> Result<(), String> {
        let (row, col) = self.parser.screen().cursor_position();
        let replies = self.answers.feed(bytes, (row + 1, col + 1));
        if replies.is_empty() {
            return Ok(());
        }
        self.writer
            .write_all(&replies)
            .and_then(|()| self.writer.flush())
            .map_err(|e| format!("could not answer a terminal query: {e}"))
    }

    /// The last [`RAW_TAIL`] bytes as printable escapes, so a CI log shows
    /// what ConPTY or the pty actually sent when the parsed screen is empty.
    fn raw_tail(&self) -> String {
        let start = self.raw.len().saturating_sub(RAW_TAIL);
        let mut out = String::new();
        for byte in &self.raw[start..] {
            match byte {
                b'\n' => out.push_str("\\n\n"),
                b'\r' => out.push_str("\\r"),
                b'\x1b' => out.push_str("\\e"),
                0x20..=0x7e => out.push(*byte as char),
                _ => out.push_str(&format!("\\x{byte:02x}")),
            }
        }
        out
    }
}

/// Renders the parsed screen as text rows, trailing whitespace trimmed so an
/// assertion never depends on column padding.
fn screen_text(screen: &vt100::Screen) -> String {
    let mut out = String::new();
    let cols = screen.size().1;
    for row in screen.rows(0, cols) {
        out.push_str(row.trim_end());
        out.push('\n');
    }
    out
}
