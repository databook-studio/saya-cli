//! What one run measures, where it is written, and the median summary.
//!
//! One line of JSON per run goes to `target/perf/<label>.jsonl`; the medians
//! across the runs go to stdout. Nothing here interprets a number — the pass
//! bars live in `scripts/perf-compare.sh`, which reads this file.

use std::path::Path;

use serde::Serialize;

/// Every measure the harness reports for one run of one binary. A measure it
/// could not take is `None` and its reason is in `skipped` — a number is never
/// invented.
#[derive(Debug, Default, Serialize)]
pub(crate) struct RunRecord {
    pub(crate) label: String,
    pub(crate) bin: String,
    pub(crate) run: usize,
    pub(crate) k: usize,
    /// The steady turns measured, and the provider delay subtracted from each.
    pub(crate) turns: usize,
    pub(crate) provider_delay_ms: f64,
    /// (a) send → the first streamed event of the first message.
    pub(crate) first_event_ms: f64,
    /// (b) steady turn overhead beyond the scripted provider, p50/p95.
    pub(crate) overhead_p50_ms: f64,
    pub(crate) overhead_p95_ms: f64,
    /// (c) request bytes per turn, median, from the mock's own capture.
    pub(crate) request_bytes_median: f64,
    /// (d) idle CPU of each front over the window, as a percentage of it.
    pub(crate) idle_cpu_line_pct: Option<f64>,
    pub(crate) idle_cpu_tui_pct: Option<f64>,
    pub(crate) idle_window_s: f64,
    /// (e) frames the TUI emitted per second while a 1 000-chunk answer
    /// streamed, and the interval between them.
    pub(crate) draws_per_second: Option<f64>,
    pub(crate) frame_interval_ms: Option<f64>,
    pub(crate) tokens_streamed: usize,
    /// (f) bytes the turn's own state writes added under the state dir.
    pub(crate) disk_bytes_per_turn: f64,
    pub(crate) skipped: Vec<String>,
}

impl RunRecord {
    /// The measure lines the summary prints, in the order the plan lists them.
    pub(crate) fn summary_lines(&self) -> Vec<(&'static str, String)> {
        let some = |value: Option<f64>, unit: &str| match value {
            Some(value) => format!("{value:.2}{unit}"),
            None => "skipped".to_owned(),
        };
        vec![
            ("first message", format!("{:.2} ms", self.first_event_ms)),
            (
                "turn overhead",
                format!(
                    "p50 {:.2} ms / p95 {:.2} ms over {} turns",
                    self.overhead_p50_ms, self.overhead_p95_ms, self.turns
                ),
            ),
            (
                "request bytes",
                format!("{:.0} per turn", self.request_bytes_median),
            ),
            ("idle CPU (line)", some(self.idle_cpu_line_pct, " %")),
            ("idle CPU (tui)", some(self.idle_cpu_tui_pct, " %")),
            (
                "draws while streaming",
                match (self.draws_per_second, self.frame_interval_ms) {
                    (Some(rate), Some(interval)) => format!(
                        "{rate:.2}/s, {interval:.2} ms apart over {} tokens",
                        self.tokens_streamed
                    ),
                    _ => "skipped".to_owned(),
                },
            ),
            (
                "disk per turn",
                format!("{:.0} bytes", self.disk_bytes_per_turn),
            ),
        ]
    }
}

/// Appends one record as a JSON line to `target/perf/<label>.jsonl`.
pub(crate) fn append(label: &str, record: &RunRecord) {
    let dir = crate::spawn::workspace_target().join("perf");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{label}.jsonl"));
    let line = serde_json::to_string(record).unwrap();
    let mut existing = std::fs::read_to_string(&path).unwrap_or_default();
    existing.push_str(&line);
    existing.push('\n');
    std::fs::write(&path, existing).unwrap();
}

/// One run's measures, as stdout shows them.
pub(crate) fn print_run(label: &str, record: &RunRecord) {
    println!(
        "perf-chat {label} run {}/{} ({})",
        record.run + 1,
        record.k,
        record.bin
    );
    for (name, value) in record.summary_lines() {
        println!("  {name:24} {value}");
    }
    for reason in &record.skipped {
        println!("  skipped                  {reason}");
    }
}

/// The medians across the runs: the summary a timed campaign is read from.
pub(crate) fn print_medians(label: &str, records: &[RunRecord]) {
    println!("perf-chat {label} medians over {} run(s)", records.len());
    let column = |value: fn(&RunRecord) -> Option<f64>| -> String {
        let values: Vec<f64> = records.iter().filter_map(value).collect();
        match values.is_empty() {
            true => "skipped".to_owned(),
            false => format!("{:.2}", median(&values)),
        }
    };
    let fields = [
        (
            "first_event_ms",
            (|r: &RunRecord| Some(r.first_event_ms)) as fn(&RunRecord) -> Option<f64>,
        ),
        ("overhead_p50_ms", |r: &RunRecord| Some(r.overhead_p50_ms)),
        ("overhead_p95_ms", |r: &RunRecord| Some(r.overhead_p95_ms)),
        ("request_bytes_median", |r: &RunRecord| {
            Some(r.request_bytes_median)
        }),
        ("idle_cpu_line_pct", |r: &RunRecord| r.idle_cpu_line_pct),
        ("idle_cpu_tui_pct", |r: &RunRecord| r.idle_cpu_tui_pct),
        ("draws_per_second", |r: &RunRecord| r.draws_per_second),
        ("frame_interval_ms", |r: &RunRecord| r.frame_interval_ms),
        ("disk_bytes_per_turn", |r: &RunRecord| {
            Some(r.disk_bytes_per_turn)
        }),
    ];
    for (name, field) in fields {
        println!("  {name:24} {}", column(field));
    }
}

/// The middle value, or the mean of the middle pair for an even count.
pub(crate) fn median(values: &[f64]) -> f64 {
    percentile(values, 50.0)
}

/// The value at `p` percent (nearest rank), over the values it is given.
pub(crate) fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap());
    let rank = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

/// Every byte under `path`, recursively — what a turn's state writes added.
pub(crate) fn tree_bytes(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => tree_bytes(&entry.path()),
            Ok(_) => entry.metadata().map(|meta| meta.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}
