use std::{fs, num::NonZeroU64, os::unix::fs::PermissionsExt};

use saya_harness::coding::commands::{CommandOwner, OutputStream};

use super::{PRIVACY_HELPER, fixture, join_exits_cleanly, wait_for_status};

/// (a) After a finished run with output, the store directory and both
/// stream files carry private modes: the store dir is 0o700 and each
/// stream file 0o600, read through symlink metadata.
#[tokio::test]
async fn store_files_are_private() {
    let Some(fixture) = fixture() else { return };
    let owner = CommandOwner::new(1).unwrap();
    let session = owner
        .start_with_output(
            fixture.run_program(),
            PRIVACY_HELPER.to_owned(),
            Vec::new(),
            NonZeroU64::new(20).unwrap(),
            fixture.marker("privacy-perms"),
        )
        .unwrap();
    wait_for_status(&session).await;
    let store_dir = session
        .output_dir()
        .expect("session carries an output store");
    let dir_mode = fs::symlink_metadata(&store_dir)
        .expect("the store dir exists")
        .permissions()
        .mode();
    assert_eq!(dir_mode & 0o777, 0o700, "the store dir is private");
    let stdout = store_dir.join(OutputStream::Stdout.file_name());
    let stdout_mode = fs::symlink_metadata(&stdout)
        .expect("the stdout stream file exists after a run with output")
        .permissions()
        .mode();
    assert_eq!(
        stdout_mode & 0o777,
        0o600,
        "the stdout stream file is private"
    );
    let stderr = store_dir.join(OutputStream::Stderr.file_name());
    let stderr_mode = fs::symlink_metadata(&stderr)
        .expect("the stderr stream file exists after a run with output")
        .permissions()
        .mode();
    assert_eq!(
        stderr_mode & 0o777,
        0o600,
        "the stderr stream file is private"
    );
    join_exits_cleanly(session).await;
}
