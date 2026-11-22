//! Run-core parity, row C1: which tools does each surface advertise on its
//! FIRST provider request?
//!
//! The surfaces share one run core, so a tool the chat session offers should be
//! the same definition a `-p`/job root or a helper offers. This test captures
//! each surface's first request with a scripted provider, compares the ordered
//! tool names and a short schema hash, and pins today's differences in
//! [`known_gaps`]. A new difference fails the test; a closed one fails too and
//! names the row to delete.
//!
//! Surfaces not captured are stated in [`known_gaps::NOT_CAPTURED`] with the
//! reason, and printed with the matrix.

#![cfg(unix)]

use std::time::Duration;

// The three context-efficiency modules and the REPL harness are shared by
// path; this binary uses only part of each, so silence the unused remainder.
#[path = "coding_context_efficiency/fixture.rs"]
#[allow(dead_code)] // shared test harness: this target uses part of it
mod fixture;
#[path = "mvp/session_question_budget/harness.rs"]
#[allow(dead_code)] // shared test harness: this target uses part of it
mod harness;
#[path = "coding_context_efficiency/provider.rs"]
#[allow(dead_code)] // shared test harness: this target uses part of it
mod provider;
#[path = "coding_context_efficiency/script.rs"]
#[allow(dead_code)] // shared test harness: this target uses part of it
mod script;

#[path = "run_core_parity/baseline.rs"]
mod baseline;
#[path = "run_core_parity/capture.rs"]
mod capture;
#[path = "run_core_parity/chat.rs"]
mod chat;
#[path = "run_core_parity/job.rs"]
mod job;
#[path = "run_core_parity/known_gaps.rs"]
mod known_gaps;
#[path = "run_core_parity/matrix.rs"]
mod matrix;
#[path = "run_core_parity/no_database.rs"]
mod no_database;
#[path = "mvp/db_goldens/normalise.rs"]
#[allow(dead_code)] // shared test harness: this target uses part of it
mod normalise;

/// The scripted provider's own lifetime; the shared provider module reads it
/// from the crate root, exactly as the context-efficiency binary does. It
/// outlasts the job run ceiling so a slow run fails on that ceiling, not on a
/// provider that shut down first.
pub(crate) const PROVIDER_LIFETIME: Duration = Duration::from_secs(180);

#[test]
fn run_core_surfaces_advertise_the_same_tools_or_recorded_gaps() {
    let job = job::capture();
    let captures = [
        matrix::SurfaceCapture {
            name: "chat",
            advertised: chat::capture(),
        },
        matrix::SurfaceCapture {
            name: "job",
            advertised: job.job,
        },
        matrix::SurfaceCapture {
            name: "helper",
            advertised: job.helper,
        },
    ];
    let matrix = matrix::Matrix::build(&captures);
    matrix.print();
    matrix.assert_known_gaps(known_gaps::KNOWN_GAPS, known_gaps::NOT_CAPTURED);
}

/// The database text belongs to the database plugin's section, so a run that
/// activates no database plugin sends none of its words.
#[test]
fn a_run_without_a_database_sends_no_database_text() {
    let body = no_database::first_request_body();
    for word in no_database::DATABASE_WORDS {
        assert!(
            !body.contains(word),
            "a run with no active database plugin sent `{word}`: {body}"
        );
    }
}

/// The rows this unit re-measured: chat, moved onto the run core. A
/// regeneration rewrites the whole file, so every other committed row is
/// checked against the fresh measurement first — a row that moved for a
/// surface this unit did not touch is that surface's regression, and must be
/// reviewed with its own change rather than published here.
const REGENERATED: &[&str] = &["chat"];

/// Every committed row outside [`REGENERATED`] is measured exactly as it was
/// committed. Silent while no baseline is committed yet (a first write).
fn assert_unregenerated_rows_are_unchanged(measured: &baseline::Baseline) {
    let path = baseline::golden_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let committed: baseline::Baseline = serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("baseline {} is valid JSON: {error}", path.display()));
    for row in &committed.surfaces {
        if REGENERATED.contains(&row.surface.as_str()) {
            continue;
        }
        let measured = measured
            .surfaces
            .iter()
            .find(|surface| surface.surface == row.surface)
            .unwrap_or_else(|| panic!("the baseline no longer measures {}", row.surface));
        assert_eq!(
            measured, row,
            "{}'s row moved, and this unit regenerates only {REGENERATED:?}: review that \
             surface's own change before regenerating its row",
            row.surface
        );
    }
}

/// The request-size and prefix baseline: what each surface sends on its first
/// request, and whether its second request repeats that prefix.
#[test]
fn request_baseline_per_surface() {
    let chat = chat::converse();
    let job = job::converse();
    let captures = [
        baseline::Captured {
            surface: "chat",
            bodies: chat.bodies,
            roots: chat.roots,
        },
        baseline::Captured {
            surface: "job",
            bodies: job.jobs,
            roots: job.roots.clone(),
        },
        baseline::Captured {
            surface: "helper",
            bodies: job.helpers,
            roots: job.roots,
        },
    ];
    let baseline = baseline::Baseline::measure(&captures);
    baseline.print();
    baseline.assert_prefix_posture();
    assert_unregenerated_rows_are_unchanged(&baseline);
    baseline.assert_golden();
}
