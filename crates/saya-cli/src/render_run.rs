//! The run event renderer: one serde path, two framings.
//!
//! Every [`RunEvent`] the engine journals also belongs on the headless run
//! wire (`saya run` in NDJSON mode). This module is the single place the
//! wire's bytes are made: [`journal_line`] serializes the event once, and
//! Json and Ndjson render that same serialization — the framing (the
//! newline) is the only difference. Two renderers for the same data drift,
//! and the drift shows up as a benchmark harness silently parsing a field
//! that changed name; the journal, `saya run log`, and the live wire all
//! call [`journal_line`], so they cannot disagree.
//!
//! The wire is a dual-tag stream and that is deliberate: a `RunEvent` line
//! is tagged `"type"` (its serde derive), while the episode events
//! interleaved with it keep today's `TerminalEvent` envelope, tagged
//! `"event"`. The Spider benchmark harness (`bench/spider/bench.py`) reads
//! `event` — a `RunEvent` line has no such key and is inert to it, so the
//! wire the benchmark parses stays intact. A line with a `type` tag is a
//! lifecycle event; a line with an `event` tag is an episode event.
//!
//! Text is shaped here (`run_event_text`, `run_show_text`), following
//! `render_memory.rs`: wording lives in one shaper that every adapter — the
//! live wire, `reads.rs`, the slash adapters — calls, so the surfaces cannot
//! drift. Steps render one-based (`step 1` is the plan's first step); the
//! journal carries the raw index.

use crate::render::{RenderFormat, Rendered};
use crate::render_usage::usage_line;
use saya_harness::journal::{Journal, JournalWire};
use saya_types::{PauseReason, RunEvent, RunFailureCode};
use std::io::Write;
use std::sync::Arc;

/// The one serialization every JSON framing of a run event shares: the run
/// journal, `saya run log`, and the live wire all emit these bytes.
pub(crate) fn journal_line(event: &RunEvent) -> String {
    serde_json::to_string(event).unwrap_or_else(|_| format!("{event:?}"))
}

/// Renders one run event in the caller's format. Text is shaped in
/// [`run_event_text`]; Json and Ndjson share [`journal_line`] — the same
/// bytes the journal wrote, one line.
pub fn render_run_event(event: &RunEvent, format: RenderFormat) -> Rendered {
    match format {
        RenderFormat::Text => Rendered {
            stdout: run_event_text(event),
            stderr: String::new(),
        },
        RenderFormat::Json | RenderFormat::Ndjson => Rendered {
            stdout: format!("{}\n", journal_line(event)),
            stderr: String::new(),
        },
    }
}

/// Prints one run event for the live run stream, exactly as `emit` prints a
/// terminal event: data on stdout, nothing on stderr.
pub(crate) fn print_run_event(event: &RunEvent, format: RenderFormat) {
    let rendered = render_run_event(event, format);
    print!("{}", rendered.stdout);
    eprint!("{}", rendered.stderr);
    let _ = std::io::stdout().flush();
}

/// The journal wire: renders every event the run journals onto the process
/// stream, in journal write order. The wire and the durable record are the
/// same stream — the renderer renders the journal's own write, so it cannot
/// drift from it or duplicate it.
pub(crate) fn run_wire(format: RenderFormat) -> JournalWire {
    Arc::new(move |event: &RunEvent| print_run_event(event, format))
}

/// A journal that reports every append onto the run wire.
pub(crate) fn wired_journal(journal: Journal, format: RenderFormat) -> Journal {
    journal.with_wire(run_wire(format))
}

/// Shapes one run event's text line. Every lifecycle line says plainly what
/// happened; a paused run names its cause, because a pause is resumable and
/// the reader should know why it stopped.
pub(crate) fn run_event_text(event: &RunEvent) -> String {
    match event {
        RunEvent::RunStarted => "run started\n".into(),
        RunEvent::PlanApproved => "plan approved\n".into(),
        RunEvent::StepStarted { step } => format!("step {} started\n", step + 1),
        RunEvent::StepCompleted { step } => format!("step {} completed\n", step + 1),
        RunEvent::StepFailed { step } => format!("step {} failed\n", step + 1),
        RunEvent::Paused { reason } => format!("run paused · {}\n", pause_reason_text(*reason)),
        RunEvent::Completed => "run completed\n".into(),
        RunEvent::Failed { code } => format!("run failed: {}\n", failure_code_cause(*code)),
        RunEvent::Cancelled => "run cancelled\n".into(),
        RunEvent::Usage {
            endpoint,
            tokens,
            turns,
            tool_calls,
            cached_input_tokens,
            cache_creation_input_tokens,
        } => format!(
            "usage · {endpoint} · tokens {} · cache reads {} · cache writes {} · turns {} · tool calls {}\n",
            count_text(*tokens),
            count_text(*cached_input_tokens),
            count_text(*cache_creation_input_tokens),
            count_text(*turns),
            count_text(*tool_calls),
        ),
        // `RunEvent` is #[non_exhaustive]: a future variant this shaper does
        // not know about renders nothing rather than guess. Rendering never
        // fails the run.
        _ => String::new(),
    }
}

/// An optional count renders as its number, or `unknown` — never zero: a
/// provider that reported nothing must not be read as having cost nothing.
pub(crate) fn count_text(count: Option<u64>) -> String {
    match count {
        Some(count) => count.to_string(),
        None => "unknown".into(),
    }
}

/// A typed terminal cause's message text — the one source, shared with the
/// exit mapping (`commands::run::exit`).
pub(crate) fn failure_code_cause(code: RunFailureCode) -> &'static str {
    match code {
        RunFailureCode::SafetyQuery => "the safety gate refused a query",
        RunFailureCode::ConnectionConfig => "a connection or configuration problem",
        _ => "the provider or agent layer failed",
    }
}

/// A pause reason's message text — the one source, shared with the exit
/// mapping (`commands::run::exit`) and the show stanza below.
pub(crate) fn pause_reason_text(reason: PauseReason) -> &'static str {
    match reason {
        PauseReason::BudgetExhausted => "a declared budget tripped",
        PauseReason::WallClockExceeded => "the wall-clock budget tripped",
        PauseReason::StepFailedAfterRetry => "a step kept failing past the bounded retries",
        PauseReason::StoreUnavailable => "the state store became unavailable",
        PauseReason::UserPaused => "the run was paused by the user",
        _ => "the process holding the run died",
    }
}

/// Shapes the `saya run show` stanza — the same text `/runs <id>` renders,
/// because both adapters call the same read path that ends here. `spec` is
/// the `(goal, scopes)` pair when the run's spec file reads; a missing file
/// renders no goal or scopes line rather than failing the read.
/// Everything the run-show stanza renders. A struct rather than a positional
/// list: the call sites are the headless command and the
/// `/runs` slash adapter, and a positional list this long is exactly where
/// two callers drift by passing the same types in a different order.
pub(crate) struct RunShowStanza<'a> {
    pub id: &'a str,
    pub status: &'a str,
    pub failure_cause: Option<&'a str>,
    pub created_unix_ms: i64,
    pub updated_unix_ms: i64,
    pub spec: Option<(&'a str, &'a str)>,
    pub paused: Option<PauseReason>,
    /// Rendered here rather than by the caller so `/runs` and `saya run show`
    /// cannot diverge: the parity suite asserts they are byte-identical, and
    /// a section added at one call site only would break that silently.
    pub deliverables: &'a [String],
    /// The per-endpoint usage the run's journal records, folded per endpoint
    /// with the usage-honesty rule. A run whose journal holds no usage event
    /// renders no section — the same shape the deliverables section takes.
    pub usage: &'a [crate::render_usage::EndpointUsage],
}

pub(crate) fn run_show_text(stanza: RunShowStanza<'_>) -> String {
    let RunShowStanza {
        id,
        status,
        failure_cause,
        created_unix_ms,
        updated_unix_ms,
        spec,
        paused,
        deliverables,
        usage,
    } = stanza;
    let cause = match failure_cause {
        Some(cause) => format!(" ({cause})"),
        None => String::new(),
    };
    let mut text = format!(
        "run {id}\nstatus: {status}{cause}\ncreated: {created}\nupdated: {updated}",
        created = created_unix_ms,
        updated = updated_unix_ms,
    );
    if let Some((goal, scopes)) = spec {
        text.push_str(&format!("\ngoal: {goal}"));
        text.push_str(&format!("\nscopes: {scopes}"));
    }
    if let Some(reason) = paused {
        text.push_str(&format!("\npaused: {}", pause_reason_text(reason)));
    }
    // The artifact manifests the run recorded at its steps' completions,
    // last record per step. A run that declared none renders no section.
    if !deliverables.is_empty() {
        text.push_str("\ndeliverables:");
        for line in deliverables {
            text.push_str(&format!("\n{line}"));
        }
    }
    // What the run cost, per endpoint, as its journal recorded it — the
    // stanza's closing line, because cost is what a finished run's reader
    // asks last. A run whose journal holds no usage event renders no
    // section: nothing reported is not the same claim as costing nothing.
    if !usage.is_empty() {
        text.push_str("\nusage:");
        for entry in usage {
            text.push_str(&format!("\n{}", usage_line(entry)));
        }
    }
    text
}

#[cfg(test)]
#[path = "render_run_tests.rs"]
mod tests;
