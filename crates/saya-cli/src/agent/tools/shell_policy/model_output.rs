//! The model-facing bound on one shell result: a result enters the model's
//! context at no more than [`MODEL_OUTPUT_CAP_CHARS`] characters across stdout
//! and stderr. A larger result keeps both ends of each stream and carries a
//! marker naming how many bytes the model did not receive.
//!
//! The private file that keeps a large result's full captured output — under
//! the job's state directory, never inside the workspace — lives in [`spool`].

use saya_harness::runner::StreamCapture;

mod spool;

pub(crate) use spool::OutputSpool;

/// The spool directory under one job's state directory. The tool that serves
/// `read`/`grep` and the shell that writes here derive the same path from the
/// same state directory, so a saved result is reachable by the run that saved
/// it and by no other.
pub(crate) fn spool_dir(state_dir: &std::path::Path) -> std::path::PathBuf {
    state_dir.join(spool::SPOOL_DIR)
}

/// The most characters of one result the model reads, across stdout and
/// stderr. The inline window the trial measured is about this size; beyond it
/// the result is carried in every later request at the trial's cost.
pub(crate) const MODEL_OUTPUT_CAP_CHARS: usize = 30_000;

/// The characters reserved for the elision marker so the bounded render never
/// exceeds its budget.
const MARKER_RESERVE_CHARS: usize = 512;

/// The longest path the marker shows. A path is operator information; a path
/// too long to state is shortened rather than allowed to consume the budget.
const MAX_MARKER_PATH_CHARS: usize = 240;

/// The result the model reads: each stream already bounded, with the marker
/// naming the drop and the saved file where the bound cut something.
pub(crate) struct Bounded {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    /// The inline bound dropped bytes from at least one stream. The tool
    /// carries this in the result's `truncated` flag, so a cut that only the
    /// model bound made is still visible to the model and the job record.
    pub(crate) truncated: bool,
}

/// Bounds one result for the model without a saved file: the same ceiling and
/// the same head-and-tail cut, with a marker that says the full output was not
/// saved. Used by a surface that has no job state directory.
pub(crate) fn bound_without_spool(stdout: &StreamCapture, stderr: &StreamCapture) -> Bounded {
    if fits(stdout, stderr) {
        return unchanged(stdout, stderr);
    }
    bound(stdout, stderr, None)
}

/// Whether the result fits the inline ceiling and can be served unchanged.
fn fits(stdout: &StreamCapture, stderr: &StreamCapture) -> bool {
    stdout.text.chars().count() + stderr.text.chars().count() <= MODEL_OUTPUT_CAP_CHARS
}

fn unchanged(stdout: &StreamCapture, stderr: &StreamCapture) -> Bounded {
    Bounded {
        stdout: stdout.text.clone(),
        stderr: stderr.text.clone(),
        truncated: false,
    }
}

/// The one bounding render: each stream gets its own share of the ceiling and
/// keeps both ends, with `saved` named in the marker where the full captured
/// output was written.
fn bound(
    stdout: &StreamCapture,
    stderr: &StreamCapture,
    saved: Option<&std::path::Path>,
) -> Bounded {
    let stdout_chars = stdout.text.chars().count();
    let stderr_chars = stderr.text.chars().count();
    // Each stream may take half the budget before the other has claimed its
    // own; whatever one stream does not need is the other's.
    let mut stdout_budget = stdout_chars.min(MODEL_OUTPUT_CAP_CHARS / 2);
    let mut stderr_budget = stderr_chars.min(MODEL_OUTPUT_CAP_CHARS / 2);
    let leftover = MODEL_OUTPUT_CAP_CHARS - stdout_budget - stderr_budget;
    if stdout_chars > stdout_budget {
        stdout_budget += leftover;
    } else {
        stderr_budget += leftover;
    }
    Bounded {
        stdout: bound_stream(stdout, stdout_budget, saved),
        stderr: bound_stream(stderr, stderr_budget, saved),
        truncated: true,
    }
}

/// Bounds one stream to `budget_chars`, keeping its head and tail and placing
/// the marker between them. A stream that fits is returned unchanged.
fn bound_stream(
    capture: &StreamCapture,
    budget_chars: usize,
    saved: Option<&std::path::Path>,
) -> String {
    if capture.text.chars().count() <= budget_chars {
        return capture.text.clone();
    }
    let content = budget_chars.saturating_sub(MARKER_RESERVE_CHARS);
    let head_chars = content / 2;
    let tail_chars = content - head_chars;
    let head = take_head(&capture.text, head_chars);
    let tail = take_tail(&capture.text, tail_chars);
    let kept = (head.len() + tail.len()) as u64;
    let dropped = capture.total_bytes.saturating_sub(kept);
    let marker = marker(dropped, capture.truncated, saved);
    format!("{head}{marker}{tail}")
}

/// The marker between a bounded stream's ends: the exact bytes the model did
/// not receive, and where the full captured output went. When the capture
/// itself was ring-truncated the saved file is the captured head and tail, not
/// the whole stream, and the marker says "captured" rather than "full".
fn marker(dropped: u64, capture_truncated: bool, saved: Option<&std::path::Path>) -> String {
    let what = if capture_truncated {
        "captured output"
    } else {
        "full output"
    };
    match saved {
        Some(path) => format!(
            "\n... [{dropped} bytes elided; {what} saved to {}; read it with read/grep] ...\n",
            display_path(path)
        ),
        None => format!("\n... [{dropped} bytes elided; {what} not saved] ...\n"),
    }
}

/// The path the marker shows: shortened at a character boundary so a long
/// state path cannot consume the inline budget.
fn display_path(path: &std::path::Path) -> String {
    let text = path.display().to_string();
    if text.chars().count() <= MAX_MARKER_PATH_CHARS {
        return text;
    }
    let mut end = 0;
    for (index, _) in text.char_indices().take(MAX_MARKER_PATH_CHARS) {
        end = index;
    }
    format!("...{}", &text[end..])
}

fn take_head(text: &str, chars: usize) -> &str {
    match text.char_indices().nth(chars) {
        Some((index, _)) => &text[..index],
        None => text,
    }
}

fn take_tail(text: &str, chars: usize) -> &str {
    let total = text.chars().count();
    if chars >= total {
        return text;
    }
    match text.char_indices().nth(total - chars) {
        Some((index, _)) => &text[index..],
        None => text,
    }
}

#[cfg(test)]
#[path = "model_output_tests.rs"]
mod model_output_tests;
