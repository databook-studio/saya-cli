/// Spawns the verbose TUI child on a fresh pty WITHOUT killing it after the
/// paint: the caller quits it normally and the raw stream drains to EOF.
fn spawn_raw_tui(home: &Path) -> Result<RawSession, String> {
    let config = home.join("config.toml");
    let connections = home.join("connections.toml");
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
    Ok(RawSession {
        writer,
        parser: vt100::Parser::new(ROWS, COLS, 0),
        rx,
        raw: Vec::new(),
        child: ChildGuard::new(child),
    })
}

/// A live TUI child whose every raw byte is accumulated: the ordering proof
/// runs over `raw`, and the parsed screen only steers the waits.
struct RawSession {
    writer: Box<dyn Write + Send>,
    parser: vt100::Parser,
    rx: std::sync::mpsc::Receiver<std::io::Result<Vec<u8>>>,
    raw: Vec<u8>,
    child: ChildGuard,
}
impl RawSession {
    /// The parsed screen as text rows joined by newline characters.
    fn screen(&self) -> String {
        let mut out = String::new();
        for row in self.parser.screen().rows(0, COLS) {
            out.push_str(row.trim_end());
            out.push(char::from(10u8));
        }
        out
    }

    /// Writes raw bytes to the pty; the TUI reads them as key events.
    fn send(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).expect("pty write");
        self.writer.flush().expect("pty flush");
    }
    /// One bounded read: feeds `raw` and the parser; true exactly at EOF.
    fn pump(&mut self) -> Result<bool, String> {
        match self.rx.recv_timeout(READ_TIMEOUT) {
            Ok(Ok(bytes)) if bytes.is_empty() => Ok(true),
            Ok(Ok(bytes)) => {
                self.raw.extend_from_slice(&bytes);
                self.parser.process(&bytes);
                Ok(false)
            }
            Ok(Err(e)) => Err(format!("pty read failed: {e}")),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Ok(false),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Ok(true),
        }
    }
    /// Boundedly waits until the screen shows `needle`.
    fn wait_for(&mut self, needle: &str, within: Duration) {
        let start = Instant::now();
        loop {
            if self.screen().contains(needle) {
                return;
            }
            assert!(
                start.elapsed() < within,
                "never saw {needle:?} in time; screen: {}",
                self.screen()
            );
            match self.pump() {
                Ok(true) => panic!("the TUI exited before {needle:?} appeared"),
                Ok(false) => {}
                Err(e) => panic!("{e}"),
            }
        }
    }
    /// Boundedly waits until `needle` has left the screen.
    fn wait_gone(&mut self, needle: &str, within: Duration) {
        let start = Instant::now();
        loop {
            if !self.screen().contains(needle) {
                return;
            }
            assert!(
                start.elapsed() < within,
                "{needle:?} never left; screen: {}",
                self.screen()
            );
            match self.pump() {
                Ok(true) => panic!("the TUI exited while waiting for {needle:?} to leave"),
                Ok(false) => {}
                Err(e) => panic!("{e}"),
            }
        }
    }
}
