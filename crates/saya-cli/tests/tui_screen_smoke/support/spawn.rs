// Spawning the real binary on a native pty, and the reader thread that turns
// the master into a channel the waits can poll with a deadline.
//
// The master handle is returned inside `Tui` and held for the whole session:
// on Windows it owns the ConPTY, and dropping it closes the console under the
// running child.

/// Launches the real `saya` binary on a pty of [`COLS`]×[`ROWS`], with the
/// offline smoke config and a scratch HOME no real state can leak into.
fn launch(home: &Path) -> Result<Tui, String> {
    let (config, connections) = write_config(home);

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
    // The child inherited the slave; the master keeps the session open.
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

    Ok(Tui {
        writer,
        parser: vt100::Parser::new(ROWS, COLS, 0),
        rx,
        _master: pair.master,
        child: ChildGuard::new(child),
        raw: Vec::new(),
    })
}

/// The reader thread: pty `Read` is blocking with no portable deadline, so a
/// dedicated thread owns it and ships chunks the caller waits on with a
/// timeout. `Ok(empty)` is EOF, `Err` a read failure.
fn reader_channel(
    reader: Box<dyn Read + Send>,
) -> std::sync::mpsc::Receiver<std::io::Result<Vec<u8>>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = vec![0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    let _ = tx.send(Ok(Vec::new()));
                    break;
                }
                Ok(n) => {
                    if tx.send(Ok(buf[..n].to_vec())).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                    break;
                }
            }
        }
    });
    rx
}

/// Resolves the real `saya` binary cargo builds for integration tests.
fn saya_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_saya"))
}
