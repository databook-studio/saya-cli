//! The bound's arithmetic: what fits is untouched, what does not keeps both
//! ends, and the two streams share one ceiling.

use saya_harness::runner::StreamCapture;

use super::{MODEL_OUTPUT_CAP_CHARS, bound_without_spool};

fn capture(text: String) -> StreamCapture {
    StreamCapture {
        total_bytes: text.len() as u64,
        text,
        truncated: false,
        dropped_bytes: 0,
    }
}

#[test]
fn a_result_that_fits_is_unchanged() {
    let stdout = capture("hello\n".into());
    let stderr = capture("warn\n".into());
    let bounded = bound_without_spool(&stdout, &stderr);
    assert_eq!(bounded.stdout, "hello\n");
    assert_eq!(bounded.stderr, "warn\n");
    assert!(!bounded.truncated, "a result that fits was not cut");
}

#[test]
fn the_inline_bound_flags_the_cut_it_makes() {
    let stdout = capture("x".repeat(MODEL_OUTPUT_CAP_CHARS + 1));
    let bounded = bound_without_spool(&stdout, &capture(String::new()));
    assert!(
        bounded.truncated,
        "the tool must carry the inline cut in `truncated`"
    );
    assert!(bounded.stdout.contains("bytes elided"));
}

#[test]
fn a_large_result_keeps_both_ends_and_names_the_drop() {
    let text: String = "abcdefghij".repeat(10_000);
    let stdout = capture(text.clone());
    let bounded = bound_without_spool(&stdout, &capture(String::new()));
    assert!(bounded.stdout.chars().count() <= MODEL_OUTPUT_CAP_CHARS);
    assert!(bounded.stdout.starts_with("abcdefghij"));
    assert!(bounded.stdout.ends_with("abcdefghij"));
    assert!(
        bounded
            .stdout
            .contains("bytes elided; full output not saved")
    );
    let marker_start = bounded.stdout.find("\n... [").unwrap();
    let marker_end = marker_start + bounded.stdout[marker_start..].find("] ...\n").unwrap() + 6;
    let kept = (bounded.stdout.len() - (marker_end - marker_start)) as u64;
    let dropped = text.len() as u64 - kept;
    let named = bounded
        .stdout
        .split("... [")
        .nth(1)
        .and_then(|rest| rest.split(" bytes elided").next())
        .unwrap();
    assert_eq!(named, dropped.to_string());
}

/// A saved result names the path the model can open and says which tools
/// reach it, so the elision line is an instruction rather than a dead end.
#[test]
fn a_saved_result_names_its_path_and_how_to_read_it() {
    let stdout = capture("x".repeat(MODEL_OUTPUT_CAP_CHARS + 1));
    let saved = std::path::Path::new("/state/shell-output/shell-1-1.log");
    let bounded = super::bound(&stdout, &capture(String::new()), Some(saved));
    assert!(
        bounded.stdout.contains(
            "full output saved to /state/shell-output/shell-1-1.log; read it with read/grep"
        ),
        "the marker names the saved file and the tools: {}",
        bounded.stdout
    );
}

#[test]
fn the_ceiling_is_shared_across_streams() {
    let stdout = capture("o".repeat(20_000));
    let stderr = capture("e".repeat(20_000));
    let bounded = bound_without_spool(&stdout, &stderr);
    assert!(
        bounded.stdout.chars().count() + bounded.stderr.chars().count() <= MODEL_OUTPUT_CAP_CHARS,
        "stdout {} + stderr {} exceeds the ceiling",
        bounded.stdout.chars().count(),
        bounded.stderr.chars().count()
    );
    assert!(bounded.stdout.contains("bytes elided"));
    assert!(bounded.stderr.contains("bytes elided"));
}

#[test]
fn a_multibyte_result_is_cut_at_a_character_boundary() {
    let stdout = capture("é".repeat(40_000));
    let bounded = bound_without_spool(&stdout, &capture(String::new()));
    assert!(bounded.stdout.chars().count() <= MODEL_OUTPUT_CAP_CHARS);
    assert!(bounded.stdout.contains("bytes elided"));
}
