//! The child under test, spawned from a binary path, and the timestamped
//! capture of its output.
//!
//! The perf harness differs from the other scripted rows in two ways, and both
//! live here: the binary is `SAYA_PERF_BIN` (a control build is a different
//! file), and every read of the child's output is stamped with the instant the
//! read returned, so a turn's latency is taken from the stream rather than from
//! a polling interval.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

/// The per-run roots, all under `target/tmp-perf-chat` so a perf run leaves
/// nothing in the system temp dir and nothing to clean up by hand.
pub(crate) struct Roots {
    pub(crate) config_root: PathBuf,
    pub(crate) session_root: PathBuf,
}

impl Roots {
    /// A fresh pair of roots for `tag`; the caller spawns the fixture inside
    /// `config_root` before the child starts.
    pub(crate) fn new(tag: &str) -> Self {
        let base = workspace_target()
            .join("tmp-perf-chat")
            .join(format!("{tag}-{}", std::process::id()));
        let config_root = base.join("config");
        let session_root = base.join("sessions");
        for root in [&config_root, &session_root] {
            let _ = std::fs::remove_dir_all(root);
            std::fs::create_dir_all(root).unwrap();
        }
        std::fs::create_dir_all(config_root.join("home")).unwrap();
        std::fs::create_dir_all(config_root.join("cwd")).unwrap();
        Self {
            config_root,
            session_root,
        }
    }
}

/// The workspace's `target` directory: every artefact a perf run writes stays
/// under it (`target/perf`, `target/tmp-perf-chat`).
pub(crate) fn workspace_target() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target")
}

/// Writes the run's config and returns the environment every perf child gets:
/// the run's own roots, the scripted provider's coordinates, and nothing else
/// from the developer's shell except `PATH`.
pub(crate) fn prepare(roots: &Roots, config: &str, base_url: &str) -> Vec<(String, String)> {
    std::fs::write(config_file(roots), config).unwrap();
    let path = std::env::var_os("PATH").unwrap_or_default();
    let display = |path: PathBuf| path.display().to_string();
    vec![
        ("PATH".to_owned(), path.to_string_lossy().into_owned()),
        ("HOME".to_owned(), display(roots.config_root.join("home"))),
        (
            "SAYA_CONFIG_HOME".to_owned(),
            display(roots.config_root.clone()),
        ),
        (
            "SAYA_SESSION_DIR".to_owned(),
            display(roots.session_root.clone()),
        ),
        (
            "SAYA_STATE_DB".to_owned(),
            display(roots.config_root.join("state.sqlite3")),
        ),
        ("SAYA_PROVIDER".to_owned(), "openai_compatible".to_owned()),
        ("SAYA_MODEL".to_owned(), "mock-model".to_owned()),
        (
            "SAYA_PROVIDER_BASE_URL".to_owned(),
            format!("{base_url}/v1"),
        ),
        ("SAYA_API_KEY".to_owned(), "mock-secret".to_owned()),
    ]
}

/// Where [`prepare`] wrote the run's config.
pub(crate) fn config_file(roots: &Roots) -> PathBuf {
    roots.config_root.join("config.toml")
}

/// One output stream of the child: the bytes so far and the instant each read
/// finished, so a caller can timestamp an event without polling for it.
#[derive(Default)]
struct Capture {
    bytes: Vec<u8>,
    /// `(bytes so far, the instant that read returned)`, ascending.
    marks: Vec<(usize, Instant)>,
    closed: bool,
}

pub(crate) struct Stream {
    capture: Mutex<Capture>,
    changed: Condvar,
}

impl Stream {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            capture: Mutex::new(Capture::default()),
            changed: Condvar::new(),
        })
    }

    pub(crate) fn push(&self, chunk: &[u8]) {
        let mut capture = self.capture.lock().unwrap();
        capture.bytes.extend_from_slice(chunk);
        let end = capture.bytes.len();
        capture.marks.push((end, Instant::now()));
        drop(capture);
        self.changed.notify_all();
    }

    pub(crate) fn close(&self) {
        self.capture.lock().unwrap().closed = true;
        self.changed.notify_all();
    }

    /// The instant at which `needle` had fully arrived, or why it never did.
    pub(crate) fn wait_for(&self, needle: &[u8], within: Duration) -> Result<Instant, String> {
        let deadline = Instant::now() + within;
        let mut from: usize = 0;
        let mut capture = self.capture.lock().unwrap();
        loop {
            let start = from.saturating_sub(needle.len().saturating_sub(1));
            if let Some(end) = find(&capture.bytes[start..], needle).map(|at| start + at) {
                let at = capture
                    .marks
                    .iter()
                    .find(|(offset, _)| *offset >= end)
                    .map(|(_, instant)| *instant)
                    .unwrap_or_else(Instant::now);
                return Ok(at);
            }
            from = capture.bytes.len();
            if capture.closed {
                return Err(format!(
                    "the child's output ended before {:?} appeared; tail: {:?}",
                    String::from_utf8_lossy(needle),
                    tail(&capture.bytes)
                ));
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(format!(
                    "did not observe {:?} within {within:?}; tail: {:?}",
                    String::from_utf8_lossy(needle),
                    tail(&capture.bytes)
                ));
            }
            capture = self
                .changed
                .wait_timeout(capture, deadline - now)
                .unwrap()
                .0;
        }
    }

    /// The bytes captured so far — every byte, from the start of the process.
    pub(crate) fn bytes(&self) -> Vec<u8> {
        self.capture.lock().unwrap().bytes.clone()
    }

    /// The instant this stream first said anything: the front is up.
    pub(crate) fn wait_for_any(&self, within: Duration) -> Result<Instant, String> {
        let deadline = Instant::now() + within;
        let mut capture = self.capture.lock().unwrap();
        loop {
            if let Some((_, instant)) = capture.marks.last() {
                return Ok(*instant);
            }
            if capture.closed {
                return Err("the child's output closed without a byte".to_owned());
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(format!("the child said nothing within {within:?}"));
            }
            capture = self
                .changed
                .wait_timeout(capture, deadline - now)
                .unwrap()
                .0;
        }
    }
}

/// The last two kilobytes of a stream, as the surrounding text of a failure.
fn tail(bytes: &[u8]) -> String {
    let start = bytes.len().saturating_sub(2048);
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|at| at + needle.len())
}
