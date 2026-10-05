//! The plain REPL prints the fixed local-metrics section on stderr at each
//! settled turn: under `--verbose` only, never on stdout, and cumulative
//! across a clarification reply on the same question's collector while an
//! unrelated question starts a fresh one.

#[cfg(unix)]
use crate::session_question_budget::harness;

#[cfg(unix)]
const HEADER: &str = "saya local metrics (fixed fields; not a benchmark)";

#[cfg(unix)]
const VERBOSE_ARGS: [&str; 1] = ["--verbose"];

#[cfg(unix)]
fn metrics_config() -> String {
    "[ai]\ninvestigation_max_elapsed_seconds = 4\n".to_owned()
}

/// The section after the `index`th header: the text from the header to the
/// next blank line (each print ends with its own newline, so the section
/// closes on a blank line) or the end of stderr.
#[cfg(unix)]
fn metrics_section(stderr: &str, index: usize) -> Option<String> {
    let mut start = 0;
    for _ in 0..=index {
        let at = stderr[start..].find(HEADER)?;
        start += at + HEADER.len();
    }
    let after = &stderr[start..];
    let end = after.find("\n\n").map_or(after.len(), |at| at + 2);
    Some(after[..end].to_owned())
}

/// The `samples=` figure of one phase line inside a section, if that cell
/// recorded any sample (a never-recorded cell prints the bare word).
#[cfg(unix)]
fn phase_samples(section: &str, phase: &str) -> Option<u64> {
    let prefix = format!("phase {phase} samples=");
    section
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .and_then(|rest| rest.split(' ').next())
        .and_then(|digits| digits.parse().ok())
}

/// One natural turn, verbose or not: a prompt carrying SENTINEL-PROMPT-1 is
/// answered with SENTINEL-ANSWER-2, the child exits on `/exit`, and the
/// full stdout and stderr buffers come back after the pumps are joined.
#[cfg(unix)]
fn one_turn(tag: &str, verbose: bool) -> (Vec<u8>, Vec<u8>) {
    use std::{
        net::TcpListener,
        sync::{Arc, atomic::AtomicBool},
    };

    use harness::{
        ScriptStep, spawn_child_with_config_extra, spawn_script_server, wait_for_stream,
    };

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let release = Arc::new(AtomicBool::new(false));
    let mut server = spawn_script_server(
        listener,
        vec![ScriptStep::Summary("SENTINEL-ANSWER-2".to_owned())],
        release,
    );

    let extra: &[&str] = if verbose { &VERBOSE_ARGS } else { &[] };
    let mut spawned = spawn_child_with_config_extra(&server.address, &metrics_config(), tag, extra);
    spawned.send(b"SENTINEL-PROMPT-1\n");

    wait_for_stream(&spawned.stdout_buf, b"SENTINEL-ANSWER-2", "answer text");
    spawned.send(b"/exit\n");

    let stdout = spawned.stdout_buf.clone();
    let stderr = spawned.stderr_buf.clone();
    spawned.finish();
    server.join();
    (
        stdout.lock().unwrap().clone(),
        stderr.lock().unwrap().clone(),
    )
}

/// Waits until `sections` sections have fully arrived on the child's stderr
/// and returns the `index`th one.
#[cfg(unix)]
fn wait_for_section(spawned: &harness::SpawnedChild, index: usize, sections: usize) -> String {
    use std::time::Duration;

    harness::wait_for_stream_count(
        &spawned.stderr_buf,
        HEADER.as_bytes(),
        "metrics header",
        sections,
        Duration::from_secs(10),
    );
    // The section tail: `unrecognized` closes every print, so reaching this
    // count proves the whole section body landed before it is parsed.
    harness::wait_for_stream_count(
        &spawned.stderr_buf,
        b"unrecognized",
        "section tail",
        sections,
        Duration::from_secs(10),
    );
    let stderr_text = String::from_utf8_lossy(&spawned.stderr_buf.lock().unwrap()).into_owned();
    metrics_section(&stderr_text, index).expect("metrics section")
}
/// A `--verbose` REPL turn prints the fixed section on stderr at the
/// settled boundary: the header, the establishment and attempt phase lines
/// with numbers, and neither sentinel — the section carries only the fixed
/// field labels, never anything from the prompt or the answer.
#[cfg(unix)]
#[test]
fn a_verbose_repl_turn_prints_the_fixed_section_on_stderr() {
    let (_stdout, stderr) = one_turn("verbose-on", true);
    let stderr_text = String::from_utf8_lossy(&stderr).into_owned();
    let section = metrics_section(&stderr_text, 0).expect("first metrics section");

    assert!(
        section.contains("phase provider_establishment samples=")
            && section.contains("phase local_attempt samples="),
        "section is missing the phase lines: {section}"
    );
    let establishment = phase_samples(&section, "provider_establishment");
    assert!(
        establishment.unwrap_or(0) >= 1,
        "establishment: {establishment:?}"
    );
    let attempts = phase_samples(&section, "local_attempt");
    assert!(attempts.unwrap_or(0) >= 1, "attempts: {attempts:?}");
    assert!(
        !section.contains("SENTINEL-PROMPT-1") && !section.contains("SENTINEL-ANSWER-2"),
        "a sentinel leaked into the metrics section: {section}"
    );
}

/// Without `--verbose` no section is printed, and the plain REPL's stdout
/// is byte-identical to the verbose run's stdout for the same script: the
/// section must never touch stdout or change the normal output.
#[cfg(unix)]
#[test]
fn a_repl_without_verbose_prints_no_section() {
    let (verbose_stdout, _verbose_stderr) = one_turn("verbose-on-cmp", true);
    let (plain_stdout, plain_stderr) = one_turn("verbose-off", false);

    assert!(
        !String::from_utf8_lossy(&plain_stderr).contains(HEADER),
        "the section header must not print without --verbose"
    );
    assert_eq!(
        verbose_stdout, plain_stdout,
        "verbose stdout must stay identical to the plain REPL's stdout"
    );
}

/// The clarification's reply prints a cumulative section on the SAME
/// question's collector (the clarify turn's sample is still counted), while
/// a later unrelated question starts a fresh collector at one sample.
#[cfg(unix)]
#[test]
fn a_clarification_reply_prints_a_cumulative_section() {
    use std::{
        net::TcpListener,
        sync::{Arc, atomic::AtomicBool},
        time::Duration,
    };

    use harness::{
        ScriptStep, spawn_child_with_config_extra, spawn_script_server, wait_for_stream,
    };

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let release = Arc::new(AtomicBool::new(false));
    let mut server = spawn_script_server(
        listener,
        vec![
            ScriptStep::Clarify {
                delay: Duration::from_secs(0),
            },
            ScriptStep::NaturalWithUsage { input_tokens: 7 },
            ScriptStep::Natural,
        ],
        release,
    );

    let mut spawned = spawn_child_with_config_extra(
        &server.address,
        &metrics_config(),
        "verbose-cumulative",
        &VERBOSE_ARGS,
    );

    // Turn 1: the question clarifies; its settled section has one sample.
    spawned.send(b"which one do you mean\n");
    wait_for_stream(
        &spawned.stdout_buf,
        "⏸ question".as_bytes(),
        "pending clarification",
    );
    let first = wait_for_section(&spawned, 0, 1);
    assert_eq!(
        phase_samples(&first, "provider_establishment"),
        Some(1),
        "clarify turn establishment: {first}"
    );

    // Turn 2: the reply on the SAME question — the collector accumulated.
    spawned.send(b"option a\n");
    let second = wait_for_section(&spawned, 1, 2);
    assert_eq!(
        phase_samples(&second, "provider_establishment"),
        Some(2),
        "reply turn establishment: {second}"
    );

    // Turn 3: an unrelated question — a fresh question, a fresh collector.
    spawned.send(b"unrelated question\n");
    let third = wait_for_section(&spawned, 2, 3);
    assert_eq!(
        phase_samples(&third, "provider_establishment"),
        Some(1),
        "unrelated turn establishment: {third}"
    );

    spawned.send(b"/exit\n");
    spawned.finish();
    server.join();
}
