//! The real-subprocess proof for `saya --verbose ask`: the mock answers with
//! a prose sentinel, one run carries the global `--verbose` flag and one does
//! not, and the two runs must be indistinguishable on stdout, in exit codes,
//! and at the provider, while stderr never leaks a sentinel the turn
//! legitimately carried and the plain run prints no metrics header.
//!
//! The positive proof (the fixed metrics section appearing on stderr) needs
//! the `--verbose` gate to reach the ask subcommand; that wiring lives in
//! app.rs and is reported separately by this packet's REFERENCE report.

#[path = "verbose_metrics/harness.rs"]
mod harness;

use std::time::Duration;

use harness::{MockProvider, build_demo, run_ask_bounded, spawn_mock};

/// The prompt carries the request sentinel and a SQL-looking sentinel: both
/// reach the provider request body and must never reach stderr.
const PROMPT: &str =
    "how many orders are there? SENTINEL-PROMPT-1 SELECT count(*) FROM sentinel_orders";

/// The mock's streamed answer: reaches stdout and must never reach stderr.
const ANSWER: &str = "SENTINEL-ANSWER-2 three orders in the demo table";

/// One SSE response that speaks the answer prose and ends the stream.
fn answer_body(text: &str) -> String {
    let chunk = serde_json::json!({
        "choices": [{
            "delta": {
                "content": text,
            },
        }],
    });
    // The proper stream terminator, spelled so no single token needs escaping.
    let done = format!("data: [{}]\n\n", "DONE");
    format!("data: {chunk}\n\n{done}")
}

#[test]
fn the_verbose_ask_changes_nothing_outside_stderr_and_adds_no_leak() {
    let fixture = build_demo("verbose-metrics");
    // One scripted response per run: the mock serves exactly two connections,
    // so an extra provider call from either run fails loudly.
    let mock: MockProvider = spawn_mock(vec![answer_body(ANSWER), answer_body(ANSWER)]);

    let verbose = run_ask_bounded(
        &fixture,
        mock.address(),
        PROMPT,
        true,
        Duration::from_secs(60),
    );
    let plain = run_ask_bounded(
        &fixture,
        mock.address(),
        PROMPT,
        false,
        Duration::from_secs(60),
    );

    // Setup-refusal guard: the mock saw exactly one answering request per
    // run, each carrying the question, and the two requests are identical.
    let bodies = mock.bodies();
    assert_eq!(
        bodies.len(),
        2,
        "the mock served exactly the two answering rounds: {bodies:?}"
    );
    assert_eq!(
        bodies[0], bodies[1],
        "identical scripts see identical requests"
    );
    assert!(
        bodies[0].contains(PROMPT),
        "the answering request carries the question: {}",
        bodies[0]
    );

    assert!(
        !verbose.killed && !plain.killed,
        "both children exited on their own: verbose={:?} plain={:?}",
        verbose.code,
        plain.code
    );
    assert!(
        verbose.stdout.contains(ANSWER),
        "the answer is streamed to stdout: stdout={}",
        verbose.stdout
    );
    assert_eq!(
        verbose.code, plain.code,
        "the flag does not change the exit code: stdout={} stderr={}",
        verbose.stdout, verbose.stderr
    );
    assert_eq!(
        verbose.stdout, plain.stdout,
        "stdout is byte-identical: verbose={} plain={}",
        verbose.stdout, plain.stdout
    );
    assert!(
        !plain.stderr.contains("saya local metrics"),
        "no metrics header without --verbose: stderr={}",
        plain.stderr
    );
    let _ = std::fs::remove_dir_all(&fixture.root);
    assert_no_leaks(&plain.stderr, &verbose.stderr);
}

/// No stderr stream, plain or verbose, may leak a sentinel from the prompt,
/// the SQL-looking text, or the mock's answer: whatever `--verbose` adds to
/// stderr, it stays fixed-vocabulary diagnostics.
fn assert_no_leaks(plain_stderr: &str, verbose_stderr: &str) {
    for (name, stderr) in [("plain", plain_stderr), ("verbose", verbose_stderr)] {
        for sentinel in ["SENTINEL-PROMPT-1", "SENTINEL-ANSWER-2", "sentinel_orders"] {
            assert!(
                !stderr.contains(sentinel),
                "the sentinel {sentinel} never reaches the {name} stderr: stderr={stderr}"
            );
        }
    }
}
