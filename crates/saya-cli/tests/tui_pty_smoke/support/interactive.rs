/// How long a single command's response may take to paint. Longer than any
/// local sqlite round-trip; far under the test's own patience.
const INTERACTIVE: Duration = Duration::from_secs(15);

/// A live TUI session on a pty, kept across many interactions: the parser
/// accumulates every chunk the reader thread ships, the writer feeds lines
/// to the input buffer, and the child is killed on drop like every other
/// rung. `raw` keeps the exact byte stream (a second capture beside the
/// parser) so the setup-handoff tests can assert alternate-screen ordering.
struct InteractiveTui {
    writer: Box<dyn Write + Send>,
    parser: vt100::Parser,
    rx: std::sync::mpsc::Receiver<std::io::Result<Vec<u8>>>,
    child: ChildGuard,
    raw: Vec<u8>,
    /// How many CPR queries (`ESC[6n`) have been answered so far; the count
    /// over the accumulated raw stream decides whether another reply is due.
    cursor_replies: usize,
}

impl InteractiveTui {
    fn send(&mut self, text: &str) -> std::io::Result<()> {
        self.writer.write_all(text.as_bytes())?;
        self.writer.flush()
    }

    /// Types one line and presses Enter.
    fn send_line(&mut self, line: &str) -> std::io::Result<()> {
        self.send(&format!("{line}\r"))
    }

    fn screen(&self) -> String {
        screen_text(self.parser.screen())
    }

    fn wait_for(&mut self, needle: &str, within: Duration) -> Result<(), String> {
        let start = Instant::now();
        loop {
            if self.screen().contains(needle) {
                return Ok(());
            }
            if start.elapsed() >= within {
                return Err(format!(
                    "'{needle}' never appeared within {within:?}; screen:\n{}",
                    self.screen()
                ));
            }
            self.pump()?;
        }
    }

    fn wait_gone(&mut self, needle: &str, within: Duration) -> Result<(), String> {
        let start = Instant::now();
        loop {
            if !self.screen().contains(needle) {
                return Ok(());
            }
            if start.elapsed() >= within {
                return Err(format!(
                    "'{needle}' never left within {within:?}; screen:\n{}",
                    self.screen()
                ));
            }
            self.pump()?;
        }
    }

    /// Drains the reader channel once (bounded by READ_TIMEOUT), feeding the
    /// parser. An empty read is EOF — the child died, which is a failure
    /// with the screen as the evidence.
    fn pump(&mut self) -> Result<(), String> {
        match self.rx.recv_timeout(READ_TIMEOUT) {
            Ok(Ok(bytes)) if bytes.is_empty() => {
                Err(format!("the TUI exited; screen:\n{}", self.screen()))
            }
            Ok(Ok(bytes)) => {
                self.raw.extend_from_slice(&bytes);
                self.parser.process(&bytes);
                self.answer_cursor_query();
                Ok(())
            }
            Ok(Err(e)) => Err(format!("pty read failed: {e}")),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Ok(()),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                Err("pty reader ended".to_string())
            }
        }
    }

    /// Acts as the emulator's answer side: every unanswered CPR query in the
    /// captured raw stream gets one reply written back, built from the
    /// parser's current cursor position (1-based in the reply, 0-based in
    /// vt100). Production no longer depends on a reply, but answering any
    /// CPR a runtime path may still emit is harmless insurance.
    fn answer_cursor_query(&mut self) {
        let queries = count_of(&self.raw, CURSOR_QUERY);
        while self.cursor_replies < queries {
            let (row, col) = self.parser.screen().cursor_position();
            let reply = format!("\x1b[{};{}R", row + 1, col + 1);
            if self.writer.write_all(reply.as_bytes()).is_err() || self.writer.flush().is_err() {
                break;
            }
            self.cursor_replies += 1;
        }
    }

    fn still_running(&mut self) -> bool {
        matches!(self.child.child.try_wait(), Ok(None))
    }
}

/// Spawns the TUI for the interactive cases: the offline smoke config with
/// one writable sqlite `demo` profile (the query path must be able to
/// create its file), the same scratch-HOME isolation every rung uses.
fn interactive_tui(home: &Path) -> Result<InteractiveTui, String> {
    let config = home.join("config.toml");
    std::fs::write(&config, "[ai]\nmodel = 'smoke'\n\n[run]\nmax_rows = 10\n")
        .expect("interactive config writes");
    let connections = home.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.demo]\ntype = 'sqlite'\npath = '{}'\nread_only = false\n",
            home.join("demo.sqlite3").display()
        ),
    )
    .expect("interactive connections write");
    // The connector opens sqlite with `create_if_missing(false)`, so the
    // database file must exist before the first query. A zero-byte file is
    // a valid empty sqlite database — `SELECT 1` needs nothing more.
    std::fs::write(home.join("demo.sqlite3"), b"").expect("the demo db file is created");

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: ROWS,
            cols: COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("could not allocate pty: {e}"))?;
    let child = pair
        .slave
        .spawn_command(tui_command(home, &config, &connections))
        .map_err(|e| format!("could not spawn saya on pty: {e}"))?;
    drop(pair.slave);
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("could not take pty writer: {e}"))?;
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("could not clone pty reader: {e}"))?;
    let rx = reader_channel(reader);
    Ok(InteractiveTui {
        writer,
        parser: vt100::Parser::new(ROWS, COLS, 0),
        rx,
        child: ChildGuard::new(child),
        raw: Vec::new(),
        cursor_replies: 0,
    })
}

/// Answers the startup trust modal (`[t]rust once`) and waits until it is
/// gone, so the keys after it go to the input buffer, not the modal.
fn dismiss_trust_modal(tui: &mut InteractiveTui) {
    tui.wait_for("trust this folder?", INTERACTIVE)
        .expect("the startup trust modal is up");
    tui.send("t").expect("the trust answer");
    tui.wait_gone("trust this folder?", INTERACTIVE)
        .expect("the trust modal closed");
}
