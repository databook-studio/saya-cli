use std::{fs, num::NonZeroU64, time::Duration};

use saya_harness::coding::commands::{CommandOwner, CommandStartError};

use crate::support::{HELPER, create_private_dir, fixture, sorted_entries};

/// A pre-placed symlink at the store's own path is refused, never adopted:
/// the start fails typed and nothing appears at the link's target.
#[tokio::test]
async fn pre_placed_symlink_at_store_path_is_refused() {
    let Some(fixture) = fixture() else { return };
    let store_root = fixture.marker("store-symlink");
    create_private_dir(&store_root);
    let target = store_root.join("symlink-target");
    create_private_dir(&target);
    std::os::unix::fs::symlink(&target, store_root.join("command-output")).unwrap();
    let owner = CommandOwner::new(1).unwrap();
    let result = owner.start_with_output(
        fixture.run_program(),
        HELPER.to_owned(),
        vec!["drip".into(), "1".into()],
        NonZeroU64::new(20).unwrap(),
        store_root,
    );
    assert!(
        matches!(result, Err(CommandStartError::OutputStore)),
        "a symlink at the store path is a typed refusal"
    );
    assert!(
        fs::read_dir(&target).unwrap().next().is_none(),
        "the symlink target gains no store entries"
    );
}

/// Without a store, `RunProgram::run` is unchanged: same outcome shape,
/// null stdin, no sink on the pump seam.
#[tokio::test]
async fn no_sink_run_program_is_unchanged() {
    let Some(fixture) = fixture() else { return };
    let line = "c02-drip 0\n";
    let outcome = tokio::time::timeout(
        Duration::from_secs(20),
        fixture
            .run_program()
            .run(HELPER, &["drip".into(), "1".into()], None),
    )
    .await
    .expect("the plain run settles within 20 s")
    .unwrap();
    assert_eq!(outcome.exit_code, Some(0));
    assert_eq!(outcome.stdout.text, line);
    assert!(!outcome.stdout.truncated);
    assert_eq!(outcome.stdout.dropped_bytes, 0);
    assert_eq!(outcome.stdout.total_bytes, line.len() as u64);
    assert!(!outcome.cancelled);
    assert!(!outcome.killed_by_timeout);
    assert!(outcome.signal.is_none());
}

/// A start refused after the store is prepared — a stopped owner's
/// admission — leaves no prepared store behind: the nonce directory is
/// removed, and the root gains no entries beyond the empty command-output
/// base it carried before the call.
#[tokio::test]
async fn refused_admission_leaves_no_store_directory() {
    let Some(fixture) = fixture() else { return };
    let store_root = fixture.marker("store-refused");
    // The base exists before the call, already private, so every entry the
    // refused start would leave behind is visible as a new name.
    create_private_dir(&store_root);
    create_private_dir(&store_root.join("command-output"));
    let before = sorted_entries(&store_root);
    let owner = CommandOwner::new(1).unwrap();
    owner.cancel_all();
    let result = owner.start_with_output(
        fixture.run_program(),
        HELPER.to_owned(),
        vec!["drip".into(), "1".into()],
        NonZeroU64::new(20).unwrap(),
        store_root.clone(),
    );
    assert!(
        matches!(result, Err(CommandStartError::OwnerStopped)),
        "a stopped owner refuses admission with the typed error"
    );
    assert_eq!(
        sorted_entries(&store_root),
        before,
        "the refused start adds no new directory entries at the store root"
    );
    assert!(
        fs::read_dir(store_root.join("command-output"))
            .unwrap()
            .next()
            .is_none(),
        "the prepared nonce directory is removed on refusal"
    );
}
