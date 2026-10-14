use std::{fs, num::NonZeroU64, os::unix::fs::symlink};

use saya_harness::coding::commands::{
    CommandOutputCursor, CommandOutputError, CommandOwner, MAX_OUTPUT_READ_BYTES, OutputStream,
};

use super::{PRIVACY_HELPER, fixture, join_exits_cleanly, wait_for_status};

/// (b) A symlink pre-placed at the stdout stream path after a finished run
/// is refused on read as a typed store error: the read never serves the
/// link target's text.
#[tokio::test]
async fn pre_placed_symlink_at_stream_path_is_refused_on_read() {
    let Some(fixture) = fixture() else { return };
    let owner = CommandOwner::new(1).unwrap();
    let session = owner
        .start_with_output(
            fixture.run_program(),
            PRIVACY_HELPER.to_owned(),
            Vec::new(),
            NonZeroU64::new(20).unwrap(),
            fixture.marker("privacy-symlink"),
        )
        .unwrap();
    wait_for_status(&session).await;
    let stdout = session
        .output_dir()
        .expect("session carries an output store")
        .join(OutputStream::Stdout.file_name());
    let decoy_dir = fixture.marker("privacy-decoy");
    fs::create_dir_all(&decoy_dir).expect("the decoy dir is created");
    let decoy = decoy_dir.join(OutputStream::Stdout.file_name());
    fs::copy(&stdout, &decoy).expect("the stream copy is made");
    let copied = fs::read_to_string(&decoy).expect("the decoy copy is readable");
    assert!(
        copied.contains("c02c1-privacy-line"),
        "the decoy holds the stream's text: {copied:?}"
    );
    fs::remove_file(&stdout).expect("the stream file is removed");
    symlink(&decoy, &stdout).expect("the symlink is placed");
    let result = session.read_output(
        CommandOutputCursor::start(OutputStream::Stdout),
        MAX_OUTPUT_READ_BYTES,
    );
    assert!(
        matches!(result, Err(CommandOutputError::Store { .. })),
        "a symlink at the stream path is refused as a store error, never the target's text"
    );
    join_exits_cleanly(session).await;
}
