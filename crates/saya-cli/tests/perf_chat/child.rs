//! The line front under test: its two captured streams, a line written with a
//! timestamp, and a clean exit that still collects the trailing output.

use std::{
    io::{Read, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use crate::spawn::{Roots, Stream, config_file, prepare};

/// The line front under test, with both output streams captured.
pub(crate) struct PerfChild {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    pub(crate) out: Arc<Stream>,
    pub(crate) err: Arc<Stream>,
    pub(crate) roots: Roots,
    pumps: Vec<thread::JoinHandle<()>>,
}

impl PerfChild {
    /// Spawns `bin` against `base_url` with the run's config and arguments.
    pub(crate) fn spawn(
        bin: &Path,
        base_url: &str,
        roots: Roots,
        config: &str,
        args: &[String],
    ) -> Self {
        let env = prepare(&roots, config, base_url);
        let mut command = Command::new(bin);
        command
            .arg("--config")
            .arg(config_file(&roots))
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_clear()
            .current_dir(roots.config_root.join("cwd"));
        for (key, value) in &env {
            command.env(key, value);
        }
        let mut child = command.spawn().unwrap();
        let out = Stream::new();
        let err = Stream::new();
        let mut pumps = Vec::new();
        let streams: Vec<(Box<dyn Read + Send>, Arc<Stream>)> = vec![
            (Box::new(child.stdout.take().unwrap()), out.clone()),
            (Box::new(child.stderr.take().unwrap()), err.clone()),
        ];
        for (stream, sink) in streams {
            pumps.push(thread::spawn(move || {
                let mut reader = stream;
                let mut buffer = [0_u8; 4096];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(size) => sink.push(&buffer[..size]),
                    }
                }
                sink.close();
            }));
        }
        Self {
            stdin: child.stdin.take(),
            child: Some(child),
            out,
            err,
            roots,
            pumps,
        }
    }

    /// The child's own pid: the process whose CPU the idle measure reads. Both
    /// readers are Unix rows, so a host without per-process accounting leaves
    /// it unused.
    #[cfg(unix)]
    pub(crate) fn pid(&self) -> u32 {
        self.child.as_ref().unwrap().id()
    }

    /// Writes one line and returns the instant the write started: the start of
    /// the turn whose latency the caller measures.
    pub(crate) fn send_line(&mut self, line: &str) -> Instant {
        let stdin = self.stdin.as_mut().expect("stdin open until the run exits");
        let start = Instant::now();
        let wrote = stdin
            .write_all(line.as_bytes())
            .and_then(|()| stdin.write_all(b"\n"))
            .and_then(|()| stdin.flush());
        if let Err(error) = wrote {
            panic!(
                "the child would not take {line:?}: {error}; stderr: {:?}",
                String::from_utf8_lossy(&self.err.bytes())
            );
        }
        start
    }

    /// The child's own status once it has exited, after joining the pumps so
    /// trailing output is still captured. Removes the run's roots.
    pub(crate) fn exit(mut self) {
        if let Some(mut stdin) = self.stdin.take() {
            let _ = stdin.write_all(b"/exit\n");
            let _ = stdin.flush();
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut child = self.child.take().unwrap();
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let _ = child.wait();
        for pump in self.pumps.drain(..) {
            let _ = pump.join();
        }
        let _ = std::fs::remove_dir_all(&self.roots.config_root);
        let _ = std::fs::remove_dir_all(&self.roots.session_root);
    }
}
