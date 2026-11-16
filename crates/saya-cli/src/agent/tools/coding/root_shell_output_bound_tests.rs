//! The model-facing shell output bound: a result enters the model's context at
//! no more than 30,000 characters per call, across stdout and stderr, keeping
//! both ends. The full captured output is saved under the job's private state
//! directory, and the marker names how many bytes were dropped and where it
//! was saved.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::{
    fs,
    path::{Path, PathBuf},
};

use saya_agent::ApprovalPolicy;
use saya_harness::runner::HostPolicy;
use serde_json::{Value, json};

use super::root_shell_tests::fixture::{TempCheckout, host_call, host_shell_over_with_spool};

/// The inline ceiling the model reads.
const CAP: usize = 30_000;

fn output(result: &Value, stream: &str) -> String {
    result[stream]
        .as_str()
        .unwrap_or_else(|| panic!("`{stream}` is a string: {result}"))
        .to_owned()
}

/// The marker's span in `text`: from the opening newline through the closing
/// newline. Panics when the marker is absent.
fn marker_span(text: &str) -> (usize, usize) {
    let start = text
        .find("\n... [")
        .unwrap_or_else(|| panic!("the elision marker must be present: {text:.80}"));
    let end = start
        + text[start..]
            .find("] ...\n")
            .unwrap_or_else(|| panic!("the marker must close: {text:.80}"))
        + "] ...\n".len();
    (start, end)
}

/// The bytes the marker reports as elided.
fn dropped(text: &str) -> u64 {
    let (start, end) = marker_span(text);
    let marker = &text[start..end];
    let count = marker
        .split("... [")
        .nth(1)
        .and_then(|rest| rest.split(" bytes elided").next())
        .unwrap_or_else(|| panic!("the marker names a byte count: {marker}"));
    count.parse().expect("the elided count is a number")
}

/// The file the marker names, when it names one.
fn saved_path(text: &str) -> Option<PathBuf> {
    let (start, end) = marker_span(text);
    let marker = &text[start..end];
    let rest = marker.split("saved to ").nth(1)?;
    let path = rest
        .split(';')
        .next()
        .expect("the path is followed by how to read it");
    Some(PathBuf::from(path))
}

/// The bytes of `text` that are not the marker.
fn kept_bytes(text: &str) -> u64 {
    let (start, end) = marker_span(text);
    (text.len() - (end - start)) as u64
}

/// A command that writes exactly 200,000 bytes to stdout.
const LARGE: &str = "yes x | head -c 200000";

#[tokio::test]
async fn a_large_foreground_output_is_bounded_and_its_full_capture_is_saved() {
    let checkout = TempCheckout::new("bound-foreground");
    let spool = TempCheckout::new("bound-foreground-spool");
    let Some(shell) = host_shell_over_with_spool(
        checkout.path(),
        ApprovalPolicy::Bypass,
        None,
        HostPolicy::default(),
        Some(spool.path()),
    ) else {
        return;
    };
    let result = shell
        .call("shell", &host_call(LARGE))
        .await
        .expect("the command runs");
    let stdout = output(&result, "stdout");
    assert!(
        stdout.chars().count() <= CAP,
        "the result enters the model context at no more than {CAP} characters, got {}",
        stdout.chars().count()
    );
    assert_eq!(
        dropped(&stdout),
        200_000 - kept_bytes(&stdout),
        "the marker names exactly the bytes the model did not receive"
    );
    let path = saved_path(&stdout).expect("the marker names where the full output is saved");
    let saved = fs::read(&path).expect("the saved file exists");
    assert_eq!(
        saved.len(),
        200_000,
        "the whole output is saved, not only the inline head and tail"
    );
}

#[tokio::test]
async fn a_small_output_is_unchanged_and_saves_nothing() {
    let checkout = TempCheckout::new("bound-small");
    let spool = TempCheckout::new("bound-small-spool");
    let Some(shell) = host_shell_over_with_spool(
        checkout.path(),
        ApprovalPolicy::Bypass,
        None,
        HostPolicy::default(),
        Some(spool.path()),
    ) else {
        return;
    };
    let result = shell
        .call("shell", &host_call("printf 'hello\\n'"))
        .await
        .expect("the command runs");
    assert_eq!(output(&result, "stdout"), "hello\n");
    assert!(
        !output(&result, "stdout").contains("bytes elided"),
        "a small result carries no marker"
    );
    assert!(
        log_files(&spool.path().join("shell-output")).is_empty(),
        "a small result saves no file"
    );
}

#[tokio::test]
async fn a_large_background_output_obeys_the_same_bound_on_read() {
    let checkout = TempCheckout::new("bound-background");
    let spool = TempCheckout::new("bound-background-spool");
    let Some(shell) = host_shell_over_with_spool(
        checkout.path(),
        ApprovalPolicy::Bypass,
        None,
        HostPolicy::default(),
        Some(spool.path()),
    ) else {
        return;
    };
    let started = shell
        .call(
            "shell",
            &json!({"command": LARGE, "sandbox": false, "run_in_background": true}),
        )
        .await
        .expect("the background call starts");
    let id = started["id"].as_u64().expect("an id");
    // The read does not wait; let the bounded command finish so one read sees
    // its whole captured output.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let read = shell
        .call("shell_output", &json!({"id": id}))
        .await
        .expect("the run is readable");
    let stdout = output(&read, "stdout");
    assert!(
        stdout.chars().count() <= CAP,
        "a background read obeys the same bound, got {} characters",
        stdout.chars().count()
    );
    assert_eq!(dropped(&stdout), 200_000 - kept_bytes(&stdout));
    let path = saved_path(&stdout).expect("the marker names where the full output is saved");
    assert_eq!(
        fs::read(&path).expect("the saved file exists").len(),
        200_000
    );
}

/// The files the spool has written, when the directory exists.
fn log_files(spool: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(spool) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect()
}
