//! The request-size and cache-prefix baseline, one row per surface.
//!
//! Moving chat onto the run core changes what chat sends to the model:
//! unused tool text and a changing request prefix are known to slow Saya and
//! raise cost. This module measures today's numbers so a later chunk can
//! compare against them.
//!
//! Every surface is scripted to make one tool call, so a SECOND request
//! follows the tool result. Each row records the first request's byte length,
//! how many tools it advertises and what those definitions cost, what its
//! system message(s) cost, and whether the second request repeats the first
//! request's prefix byte for byte — the shape a provider's prompt cache
//! needs.
//!
//! Bodies are normalised (temp roots, home prefix, user name) *before* they
//! are measured, so the golden is byte-identical on every machine and run.
//! Lengths are bytes of that normalised text; the tool and system figures are
//! bytes of their compact serialised form.
//!
//! Regenerate deliberately with `SAYA_UPDATE_GOLDENS=1 cargo nextest run -p
//! saya-cli --locked --test run_core_parity`, then review the diff like code:
//! a moved number is a change in what the model is sent. Set
//! `SAYA_PARITY_DUMP_DIR` to a directory to also write each surface's
//! normalised first body there for a diff; the measurement never reads it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::normalise::Normaliser;

/// The surfaces whose second request does NOT repeat the first request's
/// prefix today, with the reason. Every other surface must stay stable. A row
/// here that has become stable fails the test, so it is deleted rather than
/// left to rot.
pub(super) const KNOWN_UNSTABLE: &[(&str, &str)] = &[];

/// One surface's first request, measured after normalisation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Surface {
    pub(super) surface: String,
    /// Bytes of the whole normalised first request body.
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

    pub(super) fn print(&self) {
        println!(
            "request baseline per surface (bytes after normalisation; prefix = request 2 \
             repeats request 1)"
        );
        println!(
            "{:<10}{:>20}{:>11}{:>12}{:>13}  prefix_stable",
            "surface", "first_request_bytes", "tool_count", "tools_bytes", "system_bytes"
        );
        for surface in &self.surfaces {
            println!(
                "{:<10}{:>20}{:>11}{:>12}{:>13}  {}",
                surface.surface,
                surface.first_request_bytes,
                surface.tool_count,
                surface.tools_bytes,
                surface.system_bytes,
                surface.prefix_stable
            );
        }
        for (surface, reason) in KNOWN_UNSTABLE {
            println!("known unstable: {surface} — {reason}");
        }
    }

    /// A surface stable today must stay stable, and a recorded gap must still
    /// be a gap: the table is the contract, as in `known_gaps`.
    pub(super) fn assert_prefix_posture(&self) {
        for surface in &self.surfaces {
            let recorded = KNOWN_UNSTABLE
                .iter()
                .find(|(name, _)| *name == surface.surface);
            match (surface.prefix_stable, recorded) {
                (true, None) | (false, Some(_)) => {}
                (false, None) => panic!(
                    "{}'s second request no longer repeats its first request's prefix: record \
                     it in baseline::KNOWN_UNSTABLE with the reason, or fix the surface",
                    surface.surface
                ),
                (true, Some((name, _))) => {
                    panic!("{name}'s prefix is stable again: delete its KNOWN_UNSTABLE row")
                }
            }
        }
        for (surface, reason) in KNOWN_UNSTABLE {
            assert!(
                self.surfaces.iter().any(|row| row.surface == *surface),
                "KNOWN_UNSTABLE names {surface}, which the baseline does not measure"
            );
            assert!(!reason.is_empty(), "KNOWN_UNSTABLE needs a reason");
        }
    }

    /// Compares against the committed baseline, or rewrites it when
    /// `SAYA_UPDATE_GOLDENS=1`. The update is explicit so a moved number is
    /// reviewed, never silently accepted.
    pub(super) fn assert_golden(&self) {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/run_core_parity/golden/request_baseline.json");
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
            self,
            &committed,
            "what the model is sent moved for one or more surfaces in {}: review the diff, then \
             regenerate with SAYA_UPDATE_GOLDENS=1 only if the change is intended",
            path.display()
        );
    }
}

impl Surface {
    fn measure(captured: &Captured) -> Self {
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
        Self {
            surface: captured.surface.to_owned(),
            first_request_bytes: normaliser.normalise(&captured.bodies[0]).len(),
            tool_count: list(&first, "tools").len(),
            tools_bytes: serial(&first["tools"]).len(),
            system_bytes: system(&first).iter().map(Vec::len).sum(),
            prefix_stable: repeats_prefix(&first, &second),
        }
    }
}

/// Writes one surface's normalised first request body under `dir` when
/// `SAYA_PARITY_DUMP_DIR` names a directory. The file name carries the
/// surface, the process id and a nanosecond stamp, so a loop of runs —
/// concurrent or not — keeps every sample and two samples can be diffed to
/// name the field that moved. The measurement itself never reads the dump.
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

fn list<'a>(body: &'a serde_json::Value, key: &str) -> &'a [serde_json::Value] {
    body[key].as_array().map(Vec::as_slice).unwrap_or_default()
}

/// The compact serialised form of one value; object keys are sorted, so two
/// requests that sent the same content compare equal whatever their wire
/// order.
fn serial(value: &serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(value).expect("a request body serialises")
}

fn messages(body: &serde_json::Value) -> Vec<Vec<u8>> {
    list(body, "messages").iter().map(serial).collect()
}

fn system(body: &serde_json::Value) -> Vec<Vec<u8>> {
    list(body, "messages")
        .iter()
        .filter(|message| message["role"] == "system")
        .map(serial)
        .collect()
}
