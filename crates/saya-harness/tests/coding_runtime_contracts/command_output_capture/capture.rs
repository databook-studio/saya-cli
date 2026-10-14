//! C04a: whole-stream raw capture under the real supervised session owner.
use super::cursor::{join_exits_cleanly, wait_for_status};
use crate::support::{HELPER, fixture};
use saya_harness::coding::commands::{
    CommandOutputError, CommandOwner, MAX_OUTPUT_READ_BYTES, OutputStream,
};
use std::{
    num::NonZeroU64,
    time::{Duration, Instant},
};

#[tokio::test]
async fn complete_capture_preserves_utf8_across_cursor_window_boundaries() {
    let fixture = fixture().expect("capture acceptance requires proven host confinement");
    let text = format!("{}é-c04-byte-probe", "x".repeat(MAX_OUTPUT_READ_BYTES - 1));
    let owner = CommandOwner::new(1).unwrap();
    let session = owner
        .start_with_output(
            fixture.run_program(),
            HELPER.to_owned(),
            vec!["echo-bytes".into(), text.clone()],
            NonZeroU64::new(30).unwrap(),
            fixture.marker("capture-utf8"),
        )
        .unwrap();
    let status = wait_for_status(&session).await;
    assert!(status.error.is_none() && !status.capped && status.dropped_bytes == 0);
    let capture = session.capture_output(OutputStream::Stdout).unwrap();
    assert_eq!(capture.stream(), OutputStream::Stdout);
    assert_eq!(capture.as_bytes(), text.as_bytes());
    assert_eq!(std::str::from_utf8(capture.as_bytes()).unwrap(), text);
    assert!(!format!("{capture:?}").contains("c04-byte-probe"));
    assert!(!capture.directory_id().is_empty());
    join_exits_cleanly(session).await;
    // Capture owns its bytes after the session is consumed, without reopening a path.
    assert_eq!(capture.as_bytes(), text.as_bytes());
}

#[tokio::test]
async fn incomplete_and_storeless_sessions_refuse_capture_with_typed_causes() {
    let fixture = fixture().expect("capture acceptance requires proven host confinement");
    let owner = CommandOwner::new(1).unwrap();
    let pid = fixture.marker("capture-held-pid");
    let held = owner
        .start_with_output(
            fixture.run_program(),
            HELPER.to_owned(),
            vec!["hold".into(), pid.to_string_lossy().into_owned()],
            NonZeroU64::new(40).unwrap(),
            fixture.marker("capture-held"),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !pid.exists() {
        assert!(
            Instant::now() < deadline,
            "helper must enter held process state"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        !held.is_finished(),
        "the refusal must exercise unfinished writers"
    );
    assert!(matches!(
        held.capture_output(OutputStream::Stdout),
        Err(CommandOutputError::OutputNotComplete)
    ));
    let outcome = tokio::time::timeout(Duration::from_secs(10), held.stop_and_join())
        .await
        .unwrap()
        .unwrap();
    assert!(outcome.cancelled);
    let plain = owner
        .start(
            fixture.run_program(),
            HELPER.to_owned(),
            vec!["drip".into(), "1".into()],
            NonZeroU64::new(30).unwrap(),
        )
        .unwrap();
    assert!(matches!(
        plain.capture_output(OutputStream::Stdout),
        Err(CommandOutputError::Unavailable)
    ));
    join_exits_cleanly(plain).await;
}

#[tokio::test]
async fn capped_output_is_not_an_authoritative_complete_capture() {
    let fixture = fixture().expect("capture acceptance requires proven host confinement");
    let owner = CommandOwner::new(1).unwrap();
    let session = owner
        .start_with_output(
            fixture.run_program(),
            HELPER.to_owned(),
            vec!["flood".into(), "1".into(), "0".into()],
            NonZeroU64::new(30).unwrap(),
            fixture.marker("capture-capped"),
        )
        .unwrap();
    let status = wait_for_status(&session).await;
    assert!(
        status.capped || status.dropped_bytes > 0,
        "the control must reach a real output bound"
    );
    assert!(matches!(
        session.capture_output(OutputStream::Stdout),
        Err(CommandOutputError::OutputNotComplete)
    ));
    join_exits_cleanly(session).await;
}

#[tokio::test]
async fn file_replacement_or_rewrite_does_not_replace_writer_capture() {
    let fixture = fixture().expect("capture acceptance requires proven host confinement");
    let owner = CommandOwner::new(1).unwrap();
    for replace in [false, true] {
        let original = "c04-original-output";
        let forged = "c04-tampered-output";
        assert_eq!(original.len(), forged.len());
        let session = owner
            .start_with_output(
                fixture.run_program(),
                HELPER.to_owned(),
                vec!["echo-bytes".into(), original.into()],
                NonZeroU64::new(30).unwrap(),
                fixture.marker("capture-tamper"),
            )
            .unwrap();
        wait_for_status(&session).await;
        let path = session.output_dir().unwrap().join("stdout");
        if replace {
            std::fs::remove_file(&path).unwrap();
        }
        std::fs::write(&path, forged).unwrap();
        // The private disk file is genuinely different, with identical length.
        assert_eq!(std::fs::read(&path).unwrap(), forged.as_bytes());
        let capture = session.capture_output(OutputStream::Stdout).unwrap();
        assert_eq!(capture.as_bytes(), original.as_bytes());
        join_exits_cleanly(session).await;
    }
}
