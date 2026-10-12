use std::{fs, num::NonZeroU64, time::Duration};

use saya_harness::coding::commands::{
    CommandOwner, MAX_OUTPUT_READ_BYTES, OUTPUT_STORE_CAP_BYTES, OutputStream, TRUNCATION_MARKER,
};

use crate::support::{HELPER, expected_flood_prefix, fixture};

/// The red test: flooding both streams bounds each stored file at the cap
/// plus the one truncation marker, keeps the head byte-identical to what the
/// child wrote first, and still reports the whole run honestly.
#[tokio::test]
async fn output_flood_is_bounded_and_resumable() {
    let Some(fixture) = fixture() else { return };
    let owner = CommandOwner::new(1).unwrap();
    let session = owner
        .start_with_output(
            fixture.run_program(),
            HELPER.to_owned(),
            vec!["flood".into(), "3".into(), "2".into()],
            NonZeroU64::new(30).unwrap(),
            fixture.marker("store-flood"),
        )
        .unwrap();
    let store_dir = session
        .output_dir()
        .expect("session carries an output store");
    let outcome = tokio::time::timeout(Duration::from_secs(30), session.join())
        .await
        .expect("the flood run settles within 30 s")
        .unwrap();
    assert_eq!(outcome.exit_code, Some(0), "the flood helper exits cleanly");
    let stdout_total: usize = 3 * 1024 * 1024;
    let stderr_total: usize = 2 * 1024 * 1024;
    assert_eq!(outcome.stdout.total_bytes, stdout_total as u64);
    assert_eq!(
        outcome.stdout.dropped_bytes,
        (stdout_total - OUTPUT_STORE_CAP_BYTES) as u64
    );
    assert_eq!(outcome.stderr.total_bytes, stderr_total as u64);
    assert_eq!(
        outcome.stderr.dropped_bytes,
        (stderr_total - OUTPUT_STORE_CAP_BYTES) as u64
    );
    for (stream, total, upper) in [
        (OutputStream::Stdout, stdout_total, false),
        (OutputStream::Stderr, stderr_total, true),
    ] {
        let bytes = fs::read(store_dir.join(stream.file_name()))
            .unwrap_or_else(|error| panic!("the {:?} store file reads: {error}", stream));
        assert!(
            bytes.ends_with(TRUNCATION_MARKER.as_bytes()),
            "a flooded stream ends with the one truncation marker"
        );
        let stored = &bytes[..bytes.len() - TRUNCATION_MARKER.len()];
        assert!(
            stored.len() <= OUTPUT_STORE_CAP_BYTES,
            "the stored head {} exceeds the cap",
            stored.len()
        );
        // Every pump chunk is at most MAX_OUTPUT_READ_BYTES, so the first
        // non-fitting chunk leaves the head within one chunk of the cap.
        assert!(
            stored.len() + MAX_OUTPUT_READ_BYTES > OUTPUT_STORE_CAP_BYTES,
            "the stored head is within one pump chunk of the cap: {}",
            stored.len()
        );
        assert!(
            stored.len() < total,
            "a megabyte flood cannot fit the cap: {} against {total}",
            stored.len()
        );
        assert_eq!(
            stored,
            expected_flood_prefix(stored.len(), upper).as_slice(),
            "the stored head is the stream's first bytes, byte-identical"
        );
    }
}
