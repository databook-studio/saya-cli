//! The piped-REPL child for the Q01 journey tests: write lines, wait for
//! text on stdout/stderr, finish with its captured output. The REPL runs
//! headless over piped stdin (lines are turns), `--format ndjson` keeps the
//! machine events on stdout.

use crate::harness::{Env, Scripted};
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub(crate) struct Piped {
    child: Child,
    out: Arc<Mutex<Vec<u8>>>,
    err: Arc<Mutex<Vec<u8>>>,
}

impl Piped {
    pub(crate) fn spawn(env: &Env, scripted: &Scripted, extra: &[&str]) -> Piped {
        let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
        command
            .args([
                "--workspace",
                env.project.to_str().unwrap(),
                "--approval-mode",
                "bypass",
                "--allow-data-sharing",
                "--format",
                "ndjson",
            ])
            .args(extra)
            .current_dir(&env.root)
            .env("SAYA_CONFIG_HOME", env.root.join("cfg"))
            .env("SAYA_SESSION_DIR", &env.sessions)
            .env("SAYA_STATE_DB", env.root.join("state.sqlite3"))
            .env("SAYA_PROVIDER", "openai_compatible")
            .env("SAYA_MODEL", "mock-model")
            .env("SAYA_PROVIDER_BASE_URL", format!("{}/v1", scripted.address))
            .env("SAYA_API_KEY", "mock-secret")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().unwrap();
        let out = Arc::new(Mutex::new(Vec::new()));
        let err = Arc::new(Mutex::new(Vec::new()));
        for (mut pipe, sink) in [
            (
                Box::new(child.stdout.take().unwrap()) as Box<dyn Read + Send>,
                Arc::clone(&out),
            ),
            (
                Box::new(child.stderr.take().unwrap()) as Box<dyn Read + Send>,
                Arc::clone(&err),
            ),
        ] {
            thread::spawn(move || {
                let mut buf = [0_u8; 4096];
                while let Ok(n) = pipe.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    sink.lock().unwrap().extend_from_slice(&buf[..n]);
                }
            });
        }
        Piped { child, out, err }
    }

    pub(crate) fn line(&mut self, text: &str) {
        let stdin = self.child.stdin.as_mut().unwrap();
        stdin.write_all(format!("{text}\n").as_bytes()).unwrap();
        stdin.flush().unwrap();
    }

    pub(crate) fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.out.lock().unwrap()).into_owned()
    }

    pub(crate) fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.err.lock().unwrap()).into_owned()
    }

    /// Waits until `needle` appears in stdout+stderr (deadline 20 s).
    pub(crate) fn wait_for(&self, needle: &str) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if self.stdout().contains(needle) || self.stderr().contains(needle) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "never saw {needle:?}\nstdout:\n{}\nstderr:\n{}",
                self.stdout(),
                self.stderr()
            );
            thread::sleep(Duration::from_millis(25));
        }
    }

    /// Waits until stdout carries `needle` at least `at_least` times — the
    /// per-turn completion event fires once per prose turn, so the count
    /// tells the filler turns apart.
    pub(crate) fn wait_for_count(&self, needle: &str, at_least: usize) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if self.stdout().matches(needle).count() >= at_least {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "only saw {needle:?} {} times (wanted {at_least})\nstdout:\n{}\nstderr:\n{}",
                self.stdout().matches(needle).count(),
                self.stdout(),
                self.stderr()
            );
            thread::sleep(Duration::from_millis(25));
        }
    }

    /// Ends the session (stdin EOF), waits for the exit, and returns the
    /// status with both captured streams.
    pub(crate) fn finish(mut self) -> (std::process::ExitStatus, String, String) {
        drop(self.child.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(20);
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline, "child did not exit");
            thread::sleep(Duration::from_millis(25));
        };
        (status, self.stdout(), self.stderr())
    }
}

/// Waits for the nth prose turn's completion event (1-based).
pub(crate) fn complete_events(piped: &Piped, nth: usize) {
    piped.wait_for_count("\"event\":\"complete\"", nth);
}
