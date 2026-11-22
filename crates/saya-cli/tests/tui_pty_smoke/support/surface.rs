/// The new surface's harness: the real binary on a pty, a config that names a
/// provider (when one is given), and the terminal left in the caller's hands so
/// a test can read the modes it was put back into.
struct SurfaceTui {
    writer: Box<dyn Write + Send>,
    parser: vt100::Parser,
    rx: std::sync::mpsc::Receiver<std::io::Result<Vec<u8>>>,
    /// The pty master, kept for the question `/exit` has to answer: the line
    /// discipline the child leaves behind is read from its descriptor.
    master: Box<dyn portable_pty::MasterPty + Send>,
    child: ChildGuard,
    raw: Vec<u8>,
}

impl SurfaceTui {
    fn send(&mut self, text: &str) -> std::io::Result<()> {
        self.writer.write_all(text.as_bytes())?;
        self.writer.flush()
    }

    /// Types one line and presses Enter.
    fn send_line(&mut self, line: &str) -> std::io::Result<()> {
        self.send(line)?;
        self.send("\r")
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

    /// Boundedly waits until the child process has ended, draining its output
    /// as it goes: the proof that the surface left on its own.
    fn wait_exit(&mut self, within: Duration) -> Result<(), String> {
        let start = Instant::now();
        loop {
            if matches!(self.child.child.try_wait(), Ok(Some(_))) {
                return Ok(());
            }
            if start.elapsed() >= within {
                return Err(format!(
                    "the surface never left within {within:?}; screen:\n{}",
                    self.screen()
                ));
            }
            let _ = self.pump();
        }
    }

    fn still_running(&mut self) -> bool {
        matches!(self.child.child.try_wait(), Ok(None))
    }

    /// Whether the terminal the child was given is back in canonical mode: the
    /// raw mode it drew in is gone, so an operator's shell reads normally.
    fn canonical(&self) -> bool {
        let Some(fd) = self.master.as_raw_fd() else {
            return false;
        };
        let mut termios = unsafe { std::mem::zeroed::<libc::termios>() };
        if unsafe { libc::tcgetattr(fd, &mut termios) } != 0 {
            return false;
        }
        termios.c_lflag & libc::ICANON != 0
    }

    fn pump(&mut self) -> Result<(), String> {
        match self.rx.recv_timeout(READ_TIMEOUT) {
            Ok(Ok(bytes)) if bytes.is_empty() => {
                Err(format!("the surface exited; screen:\n{}", self.screen()))
            }
            Ok(Ok(bytes)) => {
                self.raw.extend_from_slice(&bytes);
                self.parser.process(&bytes);
                Ok(())
            }
            Ok(Err(error)) => Err(format!("pty read failed: {error}")),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Ok(()),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                Err("pty reader ended".to_string())
            }
        }
    }

    /// Answers the startup trust question by continuing without a workspace
    /// (`n`) and waits until the pane is gone, so the keys after it reach the
    /// composer.
    fn dismiss_trust(&mut self) {
        self.wait_for("trust this folder?", INTERACTIVE)
            .expect("the startup trust question is up");
        self.send("n").expect("the trust answer");
        self.wait_for("Message", INTERACTIVE)
            .expect("the composer is back after the answer");
    }
}

/// Launches the surface on a pty: `offline` is the smoke config with no
/// provider answering, and `Some(provider)` points the launch at the mock.
fn surface_tui(home: &Path, provider: Option<&MockProvider>) -> Result<SurfaceTui, String> {
    surface_over(home, provider, true)
}

/// The same launch with the configured profile pointing at a SQLite file that
/// is not there: the launch that must still open — as a session with no
/// database — and say why.
fn surface_tui_without_database(
    home: &Path,
    provider: Option<&MockProvider>,
) -> Result<SurfaceTui, String> {
    surface_over(home, provider, false)
}

/// `database_present` decides whether the profile's file exists: a profile
/// naming a missing file is the stale-config case, and every other byte of the
/// launch is the same.
fn surface_over(
    home: &Path,
    provider: Option<&MockProvider>,
    database_present: bool,
) -> Result<SurfaceTui, String> {
    let config = home.join("config.toml");
    let memory = if provider.is_some() {
        "[memory]\nmode = 'off'\n"
    } else {
        ""
    };
    std::fs::write(
        &config,
        format!(
            "[ai]\nmodel = 'smoke'\nallow_data_sharing = true\n\n[run]\nmax_rows = 10\n\n{memory}"
        ),
    )
    .expect("the surface config writes");
    let connections = home.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.demo]\ntype = 'sqlite'\npath = '{}'\nread_only = false\n",
            home.join("demo.sqlite3").display()
        ),
    )
    .expect("the surface connections write");
    if database_present {
        std::fs::write(home.join("demo.sqlite3"), b"").expect("the demo db file is created");
    }

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: ROWS,
            cols: COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("could not allocate pty: {e}"))?;
    let mut command = tui_command(home, &config, &connections);
    if let Some(provider) = provider {
        command.env("SAYA_PROVIDER", "openai_compatible");
        command.env("SAYA_MODEL", "mock");
        command.env("SAYA_PROVIDER_BASE_URL", provider.base_url());
        command.env("SAYA_API_KEY", "test-only");
    }
    let child = pair
        .slave
        .spawn_command(command)
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
    Ok(SurfaceTui {
        writer,
        parser: vt100::Parser::new(ROWS, COLS, 0),
        rx,
        master: pair.master,
        child: ChildGuard::new(child),
        raw: Vec::new(),
    })
}
