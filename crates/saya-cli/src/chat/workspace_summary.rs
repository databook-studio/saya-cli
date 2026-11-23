//! The bounded workspace summary a session gathers at launch.
//!
//! A session opens over a folder and the first message usually asks about it —
//! "hi", "what is this?". Without a summary the model reads the folder with
//! tool calls before it can answer, which is the wait this module removes: the
//! summary is gathered **in the background** the moment the run opens, so the
//! first request already carries what the folder holds and a greeting costs no
//! call.
//!
//! It is gathered once, bounded on every axis, and never waited for:
//!
//! - **entries** — the top level only, at most [`MAX_ENTRIES`] of them, each a
//!   name (a directory stated with its trailing `/`). Nothing walks: a session
//!   over a folder of ten thousand files reads one directory listing and stops.
//! - **bytes** — the rendered summary is at most [`MAX_BYTES`].
//! - **time** — [`SCAN_TIME`]. A scan that runs out of time keeps what it read
//!   and says it is partial.
//!
//! What it states is what a first question needs: the top-level entries, whether
//! the folder is a Git repository (its branch and how many paths differ, when
//! the caller could read them), the instruction files the project-context lane
//! already reads, and the manifest that names the language. The body is redacted
//! where it is assembled — a file name is the user's data — and it rides the
//! untrusted lane the project instructions ride: a labelled [`ContextBlock`] on
//! the user turn, never the system prompt.
//!
//! **Never a second scan.** [`WorkspaceSummaryCell`] holds the one scan a run
//! gathers: [`WorkspaceSummaryCell::spawn`] returns immediately and the scan
//! lands in the cell when it finishes, so a first message sent before it does
//! is answered without it (`take` says nothing is ready) and is never held open
//! waiting for it.

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// The most top-level entries one summary names.
pub(crate) const MAX_ENTRIES: usize = 40;

/// The most bytes one rendered summary spends on the request.
pub(crate) const MAX_BYTES: usize = 800;

/// How long one scan may take before it keeps what it read and says `partial`.
pub(crate) const SCAN_TIME: Duration = Duration::from_secs(2);

/// The label the summary reaches the model under, on the untrusted lane.
pub(crate) const SUMMARY_LABEL: &str = "workspace-summary";

/// The line an empty folder is stated with.
pub(crate) const EMPTY_FOLDER: &str = "the folder is empty";

/// The marker a summary carries when the scan stopped at one of its bounds.
pub(crate) const PARTIAL_MARK: &str = "partial: the scan stopped at its bound";

/// The manifests a language hint is read from, with the language each names.
/// The first present one is the one stated: one line, one hint.
const MANIFESTS: [(&str, &str); 7] = [
    ("Cargo.toml", "Rust"),
    ("package.json", "JavaScript/TypeScript"),
    ("pyproject.toml", "Python"),
    ("go.mod", "Go"),
    ("pom.xml", "Java"),
    ("Gemfile", "Ruby"),
    ("requirements.txt", "Python"),
];

/// The instruction files the project-context lane reads, so the summary names
/// exactly the sources a first question can rely on.
const INSTRUCTION_FILES: [&str; 2] = [
    saya_config::PROJECT_INSTRUCTIONS_FILE,
    saya_harness::instructions::CLAUDE_INSTRUCTIONS_FILE,
];

/// The bounds one scan obeys. The defaults are the module's stated bounds; a
/// test states its own to reach a bound without a folder that huge.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    pub(crate) entries: usize,
    pub(crate) bytes: usize,
    pub(crate) time: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            entries: MAX_ENTRIES,
            bytes: MAX_BYTES,
            time: SCAN_TIME,
        }
    }
}

/// What Git says about the folder, when the repository could be read: the
/// branch it is on and how many paths differ from the index. `None` on the scan
/// means no Git fact was read at all — a plain folder, or a repository the
/// service refused — so the summary states the folder's shape without it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct GitFacts {
    /// The branch name, or `None` for a head on no branch.
    pub(crate) branch: Option<String>,
    /// How many paths differ from the index.
    pub(crate) changed: usize,
}

/// One gathered summary: the body that rides the request, and whether the scan
/// stopped at a bound before the folder was fully read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct WorkspaceSummary {
    pub(crate) body: String,
    pub(crate) partial: bool,
}

impl WorkspaceSummary {
    /// The summary as the block it rides the turn on, redacted once here:
    /// while the count of redactions is nobody's business, the redaction
    /// itself is the module's promise — a name the privacy rules refuse never
    /// reaches a model.
    pub(crate) fn block(&self) -> saya_agent::ContextBlock {
        saya_agent::ContextBlock {
            label: SUMMARY_LABEL.to_owned(),
            body: saya_types::redact(&self.body),
            truncated: self.partial,
        }
    }
}

/// The one scan a run gathers, held until a turn takes it.
///
/// [`WorkspaceSummaryCell::spawn`] starts the scan on its own thread and
/// returns at once: the caller never waits for a listing, and a first message
/// that arrives before the scan finished is answered without it.
pub(crate) struct WorkspaceSummaryCell {
    slot: Arc<Mutex<Option<WorkspaceSummary>>>,
}

impl WorkspaceSummaryCell {
    /// Starts `gather` on a background thread. The closure is the whole scan —
    /// what it reads is its own business, and the cell only carries its answer.
    /// The thread is detached and bounded by its own scan, so a run that ends
    /// while it scans leaves nothing waiting on it.
    pub(crate) fn spawn<F>(gather: F) -> Self
    where
        F: FnOnce() -> WorkspaceSummary + Send + 'static,
    {
        let slot: Arc<Mutex<Option<WorkspaceSummary>>> = Arc::default();
        let landing = Arc::clone(&slot);
        thread::spawn(move || {
            let summary = gather();
            *landing.lock().unwrap_or_else(PoisonError::into_inner) = Some(summary);
        });
        Self { slot }
    }

    /// The block the first turn carries, when the scan has finished: turn 2 and
    /// later carry none — the summary is the run's launch fact, not a per-turn
    /// one — and a scan still running carries none either.
    pub(crate) fn block_for_turn(&self, turn: u64) -> Option<saya_agent::ContextBlock> {
        match turn {
            1 => self.take().map(|summary| summary.block()),
            _ => None,
        }
    }

    /// The summary, taken once, or `None` while the scan is still running.
    pub(crate) fn take(&self) -> Option<WorkspaceSummary> {
        self.slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    /// Whether the scan has finished and its summary is waiting to be taken:
    /// the fact a test waits on, and nothing a surface reads — a surface never
    /// waits for a scan.
    #[cfg(all(test, unix))]
    pub(crate) fn ready(&self) -> bool {
        self.slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }
}

/// The bounded scan of `folder`: its top-level entries, whether it is a Git
/// repository (with `git`, when the caller could read one), the instruction
/// files present, and the manifest that names its language.
///
/// Every read is bounded: one directory listing capped at `limits.entries`, one
/// `is_file` per named file, and `limits.time` across the whole scan. A scan
/// that meets a bound keeps what it read, states what it could not read, and
/// says it is partial; a folder it could not list at all is partial too, and is
/// never called empty.
pub(crate) fn scan(folder: &Path, limits: Limits, git: Option<GitFacts>) -> WorkspaceSummary {
    let start = Instant::now();
    let mut partial = false;
    let mut entries: Vec<String> = Vec::new();
    match fs::read_dir(folder) {
        Ok(listing) => {
            for entry in listing.flatten() {
                if start.elapsed() >= limits.time || entries.len() >= limits.entries {
                    partial = true;
                    break;
                }
                partial |= start.elapsed() >= limits.time;
                // A directory is marked with its trailing `/`, so the model can
                // tell a file from a folder it may open without a call.
                let name = entry.file_name().to_string_lossy().into_owned();
                let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
                entries.push(match is_dir {
                    true => format!("{name}/"),
                    false => name,
                });
            }
        }
        // A folder that cannot be listed is stated as partial, never as empty:
        // they are different facts about the workspace.
        Err(_) => partial = true,
    }
    entries.sort();
    let mut body = String::new();
    let name = folder
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| folder.to_string_lossy().into_owned());
    body.push_str(&format!("folder: {name}\n"));
    body.push_str(&match (entries.is_empty(), partial) {
        (true, false) => format!("entries: none — {EMPTY_FOLDER}\n"),
        (true, true) => "entries: none read before the bound\n".to_owned(),
        (false, _) => format!("entries (top level): {}\n", entries.join(", ")),
    });
    if folder.join(".git").exists() {
        body.push_str(&match git {
            Some(facts) => format!(
                "git: repository on {}, {} changed {}\n",
                facts.branch.as_deref().unwrap_or("a detached head"),
                facts.changed,
                match facts.changed == 1 {
                    true => "path",
                    false => "paths",
                }
            ),
            // A repository whose state the Git service could not read is still
            // a repository: the fact the model must not have to discover.
            None => "git: repository\n".to_owned(),
        });
    }
    let present: Vec<&str> = INSTRUCTION_FILES
        .iter()
        .copied()
        .filter(|file| folder.join(file).is_file())
        .collect();
    if !present.is_empty() {
        body.push_str(&format!("instructions: {}\n", present.join(", ")));
    }
    if let Some((manifest, language)) = MANIFESTS
        .iter()
        .find(|(manifest, _)| folder.join(manifest).is_file())
    {
        body.push_str(&format!("manifest: {manifest} ({language})\n"));
    }
    if partial {
        body.push_str(PARTIAL_MARK);
        body.push('\n');
    }
    WorkspaceSummary {
        body: fit(body, limits.bytes),
        partial,
    }
}

/// The body cut to `bytes`, at a character boundary, with the cut stated.
fn fit(body: String, bytes: usize) -> String {
    if body.len() <= bytes {
        return body;
    }
    let mut cut = bytes.min(body.len());
    while cut > 0 && !body.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut fitted = body[..cut].to_owned();
    fitted.push('…');
    fitted
}

#[cfg(test)]
#[path = "workspace_summary_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "workspace_summary_cell_tests.rs"]
mod cell_tests;
