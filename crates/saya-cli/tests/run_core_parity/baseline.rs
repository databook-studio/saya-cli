//! The request-size and cache-prefix baseline, one row per surface.
//!
//! Moving chat onto the run core changes what chat sends to the model: unused
//! tool text and a changing request prefix slow Saya and raise cost, so this
//! module measures today's numbers for a later chunk to compare against.
//!
//! Every surface is scripted to make one tool call, so a SECOND request follows
//! the tool result. Each row records the first request's byte length, how many
//! tools it advertises and what those definitions cost, what its system
//! message(s) cost, and whether the second request repeats the first's prefix
//! byte for byte — the shape a provider's prompt cache needs.
//!
//! Bodies are normalised (temp roots, home prefix, user name) *before* they
//! are measured, so the golden is byte-identical on every machine and run.
//! Lengths are bytes of that normalised text; the tool and system figures are
//! bytes of their compact serialised form.
//!
//! Regenerate deliberately with `SAYA_UPDATE_GOLDENS=1 cargo nextest run -p
//! saya-cli --locked --test run_core_parity`, then review the diff like code: a
//! moved number is a change in what the model is sent. `SAYA_PARITY_DUMP_DIR`
//! also writes each surface's normalised first body under a directory, for a
//! diff.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::body_fields::{list, messages, serial, system};
use super::launch_summary;
use super::normalise::Normaliser;

/// One surface's first request, measured after normalisation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Surface {
    pub(super) surface: String,
    /// Bytes of the whole normalised first request body, with the launch
    /// summary's block taken out: whether the background scan beat the run's
    /// first message is a race (`chat::workspace_summary`), so the bytes that
    /// race decides must not be in the number. `chat` is the one surface that
    /// can carry it — a job root states its own context.
    pub(super) first_request_bytes: usize,
    /// Tool definitions the first request advertises.
    pub(super) tool_count: usize,
    /// Bytes of that `tools` array, serialised compactly.
    pub(super) tools_bytes: usize,
    /// Bytes of every `system` message, serialised compactly.
    pub(super) system_bytes: usize,
    /// True when request 2 repeats request 1's `tools` array and system
    /// message(s) byte for byte and its `messages` are request 1's plus
    /// appended entries.
    pub(super) prefix_stable: bool,
    /// How many times the first request carries the launch summary's block:
    /// `0` or `1`. Recorded, and set aside before two rows are compared,
    /// because whether it is there at all is the race the bytes leave out.
    #[serde(default)]
    pub(super) first_request_launch_summary_count: usize,
}

/// The committed baseline, one row per surface in name order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Baseline {
    pub(super) surfaces: Vec<Surface>,
}

/// One surface's captured request bodies and the per-run roots to normalise.
pub(super) struct Captured {
    pub(super) surface: &'static str,
    pub(super) bodies: Vec<String>,
    pub(super) roots: Vec<PathBuf>,
}

impl Baseline {
    pub(super) fn measure(captures: &[Captured]) -> Self {
        let mut surfaces: Vec<Surface> = captures.iter().map(Surface::measure).collect();
        surfaces.sort_by(|a, b| a.surface.cmp(&b.surface));
        Self { surfaces }
    }

    /// Compares against the committed baseline, or rewrites it when
    /// `SAYA_UPDATE_GOLDENS=1`: the update is explicit so a moved number is
    /// reviewed, never silently accepted.
    pub(super) fn assert_golden(&self) {
        let path = golden_path();
        if std::env::var("SAYA_UPDATE_GOLDENS").is_ok_and(|value| value == "1") {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            let mut text = serde_json::to_string_pretty(self).unwrap();
            text.push('\n');
            std::fs::write(&path, text).unwrap();
            return;
        }
        let committed = std::fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!(
                "missing baseline {}: {error}. Regenerate with SAYA_UPDATE_GOLDENS=1",
                path.display()
            )
        });
        let committed: Baseline = serde_json::from_str(&committed)
            .unwrap_or_else(|error| panic!("baseline {} is valid JSON: {error}", path.display()));
        assert_eq!(
            self.without_launch_presence(),
            committed.without_launch_presence(),
            "what the model is sent moved for one or more surfaces in {}: review the diff, then \
             regenerate with SAYA_UPDATE_GOLDENS=1 only if the change is intended",
            path.display()
        );
    }

    /// The same rows with every surface's launch-summary count set aside, which
    /// is what two baselines are compared on.
    pub(super) fn without_launch_presence(&self) -> Self {
        let mut cleared = self.clone();
        for surface in &mut cleared.surfaces {
            surface.first_request_launch_summary_count = 0;
        }
        cleared
    }
}

impl Surface {
    /// The same row with its launch-summary count set aside: what two rows are
    /// compared on.
    pub(super) fn without_launch_presence(&self) -> Self {
        let mut cleared = self.clone();
        cleared.first_request_launch_summary_count = 0;
        cleared
    }
}

/// The committed baseline this row compares against and rewrites.
pub(super) fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/run_core_parity/golden/request_baseline.json")
}

impl Surface {
    pub(super) fn measure(captured: &Captured) -> Self {
        let mut normaliser = Normaliser::for_run();
        for root in &captured.roots {
            normaliser.add_root(root);
        }
        let body = |index: usize| {
            let raw = captured.bodies.get(index).unwrap_or_else(|| {
                panic!(
                    "the {} surface made {} provider request(s); the baseline needs two",
                    captured.surface,
                    captured.bodies.len()
                )
            });
            let text = normaliser.normalise(raw);
            serde_json::from_str::<serde_json::Value>(&text)
                .unwrap_or_else(|error| panic!("the request body is JSON: {error}"))
        };
        if let Ok(dir) = std::env::var("SAYA_PARITY_DUMP_DIR") {
            dump_first_body(
                Path::new(&dir),
                captured.surface,
                &normaliser.normalise(&captured.bodies[0]),
            );
        }
        let first = body(0);
        let second = body(1);
        let context = format!("the {} surface", captured.surface);
        let first_text = normaliser.normalise(&captured.bodies[0]);
        Self {
            surface: captured.surface.to_owned(),
            first_request_bytes: launch_summary::bytes_without_launch_summary(&first_text, &first),
            tool_count: list(&first, "tools").len(),
            tools_bytes: serial(&first["tools"]).len(),
            system_bytes: system(&first).iter().map(Vec::len).sum(),
            prefix_stable: repeats_prefix(&first, &second),
            first_request_launch_summary_count: launch_summary::recorded_count(
                &first_text,
                &first,
                &context,
            ),
        }
    }
}

/// Writes one surface's normalised first request body under `dir` when
/// `SAYA_PARITY_DUMP_DIR` names a directory. The file name carries the
/// surface, the process id and a nanosecond stamp, so every sample of a loop of
/// runs survives and two can be diffed to name the field that moved.
fn dump_first_body(dir: &Path, surface: &str, body: &str) {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let path = dir.join(format!("{surface}-{}-{stamp}.json", std::process::id()));
    std::fs::create_dir_all(dir)
        .unwrap_or_else(|error| panic!("create {}: {error}", dir.display()));
    std::fs::write(&path, body).unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
}

/// True when `second` carries `first`'s tools and system message(s) unchanged
/// and its `messages` begin with `first`'s.
fn repeats_prefix(first: &serde_json::Value, second: &serde_json::Value) -> bool {
    let before = messages(first);
    let after = messages(second);
    !before.is_empty()
        && after.len() >= before.len()
        && after[..before.len()] == before[..]
        && serial(&first["tools"]) == serial(&second["tools"])
        && system(first) == system(second)
}
