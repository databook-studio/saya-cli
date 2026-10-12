use std::{fs, num::NonZeroU64};

use saya_harness::coding::commands::{
    CommandOutputCursor, CommandOwner, MAX_OUTPUT_READ_BYTES, OutputStream,
};

use super::{PRIVACY_HELPER, fixture, join_exits_cleanly, wait_for_status};

/// (c) A credential value carrying the literal `[redacted]` marker makes
/// the redactor refuse the secret set at construction: the stream seals
/// with no stored bytes — the stdout file stays empty — and the start
/// cursor reads empty at end-of-stream.
#[tokio::test]
async fn refused_secret_set_stores_nothing() {
    let Some(fixture) = fixture() else { return };
    let owner = CommandOwner::new(1).unwrap();
    let session = owner
        .start_with_output(
            fixture.refused_program(),
            PRIVACY_HELPER.to_owned(),
            Vec::new(),
            NonZeroU64::new(20).unwrap(),
            fixture.marker("privacy-refused"),
        )
        .unwrap();
    let status = wait_for_status(&session).await;
    assert!(
        status.capped && status.error.is_none(),
        "a refused redactor seals its stream without a store error: {status:?}"
    );
    let stdout = session
        .output_dir()
        .expect("session carries an output store")
        .join(OutputStream::Stdout.file_name());
    let len = fs::symlink_metadata(&stdout)
        .expect("the stdout stream file exists")
        .len();
    assert_eq!(len, 0, "the refused run stores no stdout bytes");
    let read = session
        .read_output(
            CommandOutputCursor::start(OutputStream::Stdout),
            MAX_OUTPUT_READ_BYTES,
        )
        .expect("reading a sealed stream at the start cursor succeeds");
    assert!(
        read.text().is_empty() && read.end_of_stream(),
        "the sealed stream reads empty at end-of-stream: {:?}",
        read.text()
    );
    join_exits_cleanly(session).await;
}
