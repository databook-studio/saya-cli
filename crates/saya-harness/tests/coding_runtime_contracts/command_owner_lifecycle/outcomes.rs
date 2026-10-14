use std::num::NonZeroU64;

use saya_harness::{
    coding::commands::{CommandOwner, CommandSessionError},
    runner::{MAX_ARG_BYTES, RunnerError},
};

use super::support::{HELPER, fixture};

#[tokio::test]
async fn nonzero_exit_stays_outcome_data_and_refusals_spawn_nothing() {
    let Some(fixture) = fixture() else {
        return;
    };
    let owner = CommandOwner::new(1).unwrap();
    let exit_marker = fixture.marker("nonzero-exit");
    let outcome = owner
        .start(
            fixture.run_program(),
            HELPER.into(),
            vec!["exit".into(), exit_marker.display().to_string(), "7".into()],
            NonZeroU64::new(10).unwrap(),
        )
        .unwrap()
        .join()
        .await
        .unwrap();
    assert_eq!(outcome.exit_code, Some(7));
    assert!(!outcome.cancelled);
    assert!(exit_marker.exists());

    let blocked_marker = fixture.marker("allowlist-refusal");
    let blocked = owner
        .start(
            fixture.run_program(),
            "blocked-helper".into(),
            vec![
                "hold".into(),
                blocked_marker.display().to_string(),
                "1".into(),
            ],
            NonZeroU64::new(10).unwrap(),
        )
        .unwrap()
        .join()
        .await;
    assert!(matches!(
        blocked,
        Err(CommandSessionError::Runner(
            RunnerError::ProgramNotAllowlisted { .. }
        ))
    ));
    assert!(!blocked_marker.exists());

    let bad_argv_marker = fixture.marker("argv-refusal");
    let bad_argv = owner
        .start(
            fixture.run_program(),
            HELPER.into(),
            vec![
                "hold".into(),
                bad_argv_marker.display().to_string(),
                "x".repeat(MAX_ARG_BYTES + 1),
            ],
            NonZeroU64::new(10).unwrap(),
        )
        .unwrap()
        .join()
        .await;
    assert!(matches!(
        bad_argv,
        Err(CommandSessionError::Runner(
            RunnerError::ArgsNotTyped { .. }
        ))
    ));
    assert!(!bad_argv_marker.exists());
}
