use std::{num::NonZeroU64, time::Duration};

use saya_harness::coding::commands::CommandOwner;

use crate::support::fixture;

#[tokio::test]
async fn legacy_command_start_and_run_keep_null_stdin_eof() {
    let Some(fixture) = fixture() else { return };
    let owner = CommandOwner::new(1).unwrap();
    let session_marker = fixture.marker("legacy-session-pid");
    let session = owner
        .start(
            fixture.run_program(),
            crate::support::HELPER.to_owned(),
            vec!["read-eof".into(), session_marker.to_str().unwrap().into()],
            NonZeroU64::new(10).unwrap(),
        )
        .unwrap();
    let session_outcome = tokio::time::timeout(Duration::from_secs(5), session.join())
        .await
        .expect("null stdin presents EOF to a command session")
        .unwrap();
    assert_eq!(session_outcome.exit_code, Some(0));
    assert_eq!(session_outcome.stdout.text.trim(), "0");

    let run_marker = fixture.marker("legacy-run-pid");
    let outcome = tokio::time::timeout(
        Duration::from_secs(5),
        fixture.run_program().run(
            crate::support::HELPER,
            &["read-eof".into(), run_marker.to_str().unwrap().into()],
            Some(10),
        ),
    )
    .await
    .expect("legacy RunProgram null stdin presents EOF")
    .unwrap();
    assert_eq!(outcome.exit_code, Some(0));
    assert_eq!(outcome.stdout.text.trim(), "0");
}
