//! The job's private output spool: where a shell result too large for the
//! model's inline bound saves its full captured output. The file is bounded per
//! file and per job and lives under the job's private state directory — never
//! inside the workspace. It is reachable by this job's own `read` and `grep`
//! through the one spool directory the tool resolves, and by no other run's.

use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

use saya_harness::runner::{SHELL_SPOOL_CAPTURE_CAP_BYTES, StreamCapture};

use super::{Bounded, bound, fits, unchanged};

/// One saved file's ceiling. The capture itself is at most one mebibyte per
/// stream, so this only guards the framing.
const MAX_SPOOL_FILE_BYTES: usize = 2 * SHELL_SPOOL_CAPTURE_CAP_BYTES + 64 * 1024;

/// One job's ceiling on saved files.
const MAX_SPOOL_FILES: usize = 16;

/// One job's ceiling on saved bytes.
const MAX_SPOOL_JOB_BYTES: usize = 8 * 1024 * 1024;

/// The subdirectory of the job's state directory the files live under.
pub(super) const SPOOL_DIR: &str = "shell-output";

/// The job's private output spool, shared by every shell over the same state
/// directory, so the per-job file and byte ceilings hold across the root and
/// its helpers.
pub(crate) struct OutputSpool {
    dir: PathBuf,
    budget: Arc<Mutex<SpoolBudget>>,
}

#[derive(Default)]
struct SpoolBudget {
    files: usize,
    bytes: usize,
}

impl OutputSpool {
    /// Opens the spool under `state_dir`, or `None` where that directory is
    /// inside the workspace. The file is the operator's escape hatch, never a
    /// second way for the model to read its own output, so a state directory
    /// the workspace contains is refused rather than written to.
    pub(crate) fn new(state_dir: &Path, workspace_root: &Path) -> Option<Self> {
        let root = fs::canonicalize(workspace_root).ok()?;
        let state = fs::canonicalize(state_dir).ok()?;
        if state.starts_with(&root) {
            return None;
        }
        let dir = state.join(SPOOL_DIR);
        if dir.starts_with(&root) {
            return None;
        }
        let budget = budgets()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(dir.clone())
            .or_default()
            .clone();
        Some(Self { dir, budget })
    }

    /// Bounds one result for the model, saving the full captured streams first
    /// when the result is too large to inline. A result that fits is returned
    /// exactly as captured, and nothing is written.
    pub(crate) fn bound(&self, stdout: &StreamCapture, stderr: &StreamCapture) -> Bounded {
        if fits(stdout, stderr) {
            return unchanged(stdout, stderr);
        }
        let saved = self.save(&stdout.text, &stderr.text);
        bound(stdout, stderr, saved.as_deref())
    }

    /// Writes the full captured streams under the job's state directory and
    /// returns the file, or `None` where the per-file or per-job bound refuses
    /// it. The file is private to the owner and never inside the workspace.
    fn save(&self, stdout: &str, stderr: &str) -> Option<PathBuf> {
        let mut content = String::with_capacity(stdout.len() + stderr.len() + 32);
        content.push_str(stdout);
        if !stderr.is_empty() {
            if !stdout.is_empty() && !stdout.ends_with('\n') {
                content.push('\n');
            }
            content.push_str("--- stderr ---\n");
            content.push_str(stderr);
        }
        let content = truncate_file(content);
        let bytes = content.len();
        let mut budget = self
            .budget
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if budget.files >= MAX_SPOOL_FILES
            || budget.bytes.saturating_add(bytes) > MAX_SPOOL_JOB_BYTES
        {
            return None;
        }
        fs::create_dir_all(&self.dir).ok()?;
        set_private(&self.dir);
        let path = self.write_unique(&content)?;
        budget.files += 1;
        budget.bytes += bytes;
        Some(path)
    }

    /// Writes `content` under a name no earlier call used. The process id and
    /// a process-wide counter keep concurrent shells apart; `create_new`
    /// makes a collision a retry rather than an overwrite.
    fn write_unique(&self, content: &str) -> Option<PathBuf> {
        for _ in 0..8 {
            let name = format!("shell-{}-{}.log", std::process::id(), next_file());
            let path = self.dir.join(name);
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                options.mode(0o600);
            }
            let Ok(mut file) = options.open(&path) else {
                continue;
            };
            file.write_all(content.as_bytes()).ok()?;
            return Some(path);
        }
        None
    }
}

/// The spools of one process, keyed by directory: every shell over the same
/// job state directory shares one file and byte budget.
fn budgets() -> &'static Mutex<HashMap<PathBuf, Arc<Mutex<SpoolBudget>>>> {
    static BUDGETS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<SpoolBudget>>>>> = OnceLock::new();
    BUDGETS.get_or_init(|| Mutex::new(HashMap::new()))
}

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

fn next_file() -> u64 {
    NEXT_FILE.fetch_add(1, Ordering::Relaxed)
}

/// Cuts `content` to [`MAX_SPOOL_FILE_BYTES`] at a character boundary, saying
/// so when it had to.
fn truncate_file(mut content: String) -> String {
    if content.len() <= MAX_SPOOL_FILE_BYTES {
        return content;
    }
    let mut end = MAX_SPOOL_FILE_BYTES;
    while !content.is_char_boundary(end) {
        end -= 1;
    }
    content.truncate(end);
    content.push_str("\n... [saved output truncated] ...\n");
    content
}

#[cfg(unix)]
fn set_private(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn set_private(_path: &Path) {}
