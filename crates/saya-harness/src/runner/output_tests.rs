//! The ring's own tests: what one stream keeps, what it drops, and what
//! the render says about the difference.

use super::*;

/// The elision marker's span in a rendered text. Panics when the marker
/// is absent, which is the failure every caller asserts against.
fn marker_span(text: &str) -> (usize, usize) {
    let start = text
        .find("\n... [")
        .expect("the elision marker must be present");
    let end = start
        + text[start..]
            .find("] ...\n")
            .expect("the marker must close")
        + "] ...\n".len();
    (start, end)
}

/// The bytes a rendered text kept of the stream: everything that is not
/// the marker.
fn retained_len(text: &str) -> u64 {
    let (start, end) = marker_span(text);
    (text.len() - (end - start)) as u64
}

#[test]
fn an_over_cap_stream_keeps_both_ends_and_reports_the_elision() {
    let alphabet = b"abcdefghijklmnopqrstuvwxyz\n";
    let stream: Vec<u8> = (0..4096)
        .map(|index| alphabet[index % alphabet.len()])
        .collect();
    let ring = OutputRing::new(256);
    ring.push(&stream);
    let (text, truncated, dropped, total) = ring.snapshot();
    assert!(
        text.as_bytes().starts_with(&stream[..8]),
        "the head of the stream is kept: {text:.40}"
    );
    assert!(
        text.as_bytes().ends_with(&stream[stream.len() - 8..]),
        "the tail of the stream is kept: {text:.40}"
    );
    assert!(truncated);
    assert_eq!(total, 4096);
    assert_eq!(
        dropped,
        total - retained_len(&text),
        "the drop is exactly what the render did not keep"
    );
    let (start, end) = marker_span(&text);
    assert_eq!(
        &text[start..end],
        format!("\n... [{dropped} bytes elided] ...\n"),
        "the marker names the exact count"
    );
    assert!(
        text.len() <= 256,
        "the render stays inside the cap: {} bytes",
        text.len()
    );
}

#[test]
fn a_cap_too_small_for_its_marker_elides_everything_and_says_so() {
    // Eight bytes cannot hold both a marker and any of the stream. The
    // ring spends them on the marker and counts every byte as dropped,
    // rather than rendering a truncated stream with no sign of the cut.
    let ring = OutputRing::new(8);
    ring.push(b"0123456789");
    let (text, truncated, dropped, total) = ring.snapshot();
    let (start, end) = marker_span(&text);
    assert_eq!((start, end), (0, text.len()), "the marker is all there is");
    assert_eq!(&text[start..end], "\n... [10 bytes elided] ...\n");
    assert!(truncated);
    assert_eq!(dropped, 10);
    assert_eq!(total, 10);
}

#[test]
fn a_secret_at_either_kept_end_of_a_truncated_stream_is_scrubbed() {
    // The head window is new model-visible surface, so the redaction gate
    // has to cover it: a credential at the very start of a runaway stream
    // must not survive the capture any more than one at the end.
    let mut stream = Vec::new();
    stream.extend_from_slice(b"TOKEN=planted-9Fq3wZ\n");
    stream.resize(4096, b'.');
    stream.extend_from_slice(b"\nKEY=planted-2Rt8yK\n");
    let ring = OutputRing::new(256);
    ring.push(&stream);
    let capture = capture(
        &ring,
        &["planted-9Fq3wZ".to_owned(), "planted-2Rt8yK".to_owned()],
    );
    assert!(capture.truncated, "the stream outgrew the cap");
    assert!(
        capture.text.contains("TOKEN=[redacted]") && capture.text.contains("KEY=[redacted]"),
        "both kept ends survive with their values scrubbed: {:?}",
        capture.text
    );
    assert!(
        !capture.text.contains("planted-9Fq3wZ") && !capture.text.contains("planted-2Rt8yK"),
        "no kept byte carries a value the runner injected: {:?}",
        capture.text
    );
}

#[test]
fn an_under_cap_stream_renders_exactly_what_was_written() {
    let ring = OutputRing::new(OUTPUT_CAP_BYTES);
    let alphabet = b"abcdefghijklmnopqrstuvwxyz\n";
    let stream: Vec<u8> = (0..OUTPUT_CAP_BYTES)
        .map(|index| alphabet[index % alphabet.len()])
        .collect();
    ring.push(&stream);
    let (text, truncated, dropped, total) = ring.snapshot();
    assert_eq!(
        text.as_bytes(),
        stream.as_slice(),
        "a stream that fits is rendered whole, with no marker and no reordering"
    );
    assert!(!truncated);
    assert_eq!(dropped, 0);
    assert_eq!(total, OUTPUT_CAP_BYTES as u64);
}

#[test]
fn a_twenty_megabyte_stream_keeps_both_ends_and_counts_the_elision() {
    const TOTAL: usize = 20 * 1024 * 1024;
    let mut stream = Vec::with_capacity(TOTAL);
    stream.extend_from_slice(b"HEAD\n");
    stream.resize(TOTAL - 6, b'.');
    stream.extend_from_slice(b"\nTAIL\n");
    let ring = OutputRing::new(OUTPUT_CAP_BYTES);
    for chunk in stream.chunks(PUMP_CHUNK) {
        ring.push(chunk);
    }
    let (text, truncated, dropped, total) = ring.snapshot();
    assert_eq!(
        total, TOTAL as u64,
        "the total is everything the child wrote"
    );
    assert!(
        text.starts_with("HEAD\n"),
        "the first bytes survive: {text:.40}"
    );
    assert!(
        text.ends_with("\nTAIL\n"),
        "the last bytes survive: {text:.40}"
    );
    let (start, end) = marker_span(&text);
    assert_eq!(
        dropped,
        total - retained_len(&text),
        "the drop is exactly what the windows did not keep"
    );
    assert!(
        dropped > 20_000_000,
        "the elision is the bulk of the stream: {dropped}"
    );
    assert_eq!(
        &text[start..end],
        format!("\n... [{dropped} bytes elided] ...\n"),
        "the marker names the exact count"
    );
    assert!(truncated);
    assert!(
        text.len() <= OUTPUT_CAP_BYTES,
        "the render stays inside the same cap: {} bytes",
        text.len()
    );
}

#[test]
fn an_underfilled_ring_reports_no_truncation() {
    let ring = OutputRing::new(8);
    ring.push(b"ab");
    let (text, truncated, dropped, total) = ring.snapshot();
    assert_eq!(text, "ab");
    assert!(!truncated);
    assert_eq!(dropped, 0);
    assert_eq!(total, 2);
}

#[test]
fn a_captured_value_the_runner_injected_is_scrubbed_even_without_a_marker() {
    // The exfiltration shape: the child echoes the
    // credential under the env name the generated config binds — no
    // marker for `redact()` to match, so the value registry is what
    // stops it.
    let ring = OutputRing::new(1024);
    ring.push(b"SAYA_RUN_EP_ORCHESTRATOR=planted-7Qk2mVn9xR4\nplain line");
    let capture = capture(&ring, &["planted-7Qk2mVn9xR4".to_owned()]);
    assert!(
        !capture.text.contains("planted-7Qk2mVn9xR4"),
        "the injected value must not survive the capture: {:?}",
        capture.text
    );
    assert!(
        capture.text.contains("SAYA_RUN_EP_ORCHESTRATOR=[redacted]"),
        "the env-name shape must survive with its value scrubbed: {:?}",
        capture.text
    );
    assert!(
        capture.text.contains("plain line"),
        "non-secret output is never destroyed: {:?}",
        capture.text
    );
}

/// The line count is cumulative across reads: a notice reports what the run
/// wrote, not what one read happened to take, and a trailing fragment counts
/// as a line until its newline arrives.
#[test]
fn line_count_survives_a_take_and_counts_a_trailing_fragment() {
    let ring = OutputRing::new(1024);
    ring.push(b"a\nb\n");
    assert_eq!(ring.lines(), 2);
    let (text, ..) = ring.take();
    assert_eq!(text, "a\nb\n");
    assert_eq!(ring.lines(), 2, "a take does not reset the count");
    ring.push(b"c");
    assert_eq!(ring.lines(), 3, "the fragment is one more line");
    ring.push(b"\n");
    assert_eq!(ring.lines(), 3, "the newline completes the same line");
}
