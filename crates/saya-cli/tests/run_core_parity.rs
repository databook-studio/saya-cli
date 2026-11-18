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
#[allow(dead_code)]
mod fixture;
#[path = "mvp/session_question_budget/harness.rs"]
#[allow(dead_code)]
mod harness;
#[path = "coding_context_efficiency/provider.rs"]
#[allow(dead_code)]
mod provider;
#[path = "coding_context_efficiency/script.rs"]
#[allow(dead_code)]
mod script;

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
