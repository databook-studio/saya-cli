use super::*;

/// 4 GiB of stdout through the ring: memory stays at the cap, the tail
/// is the last thing the child wrote, and the truncation is reported —
/// then a real child flooding 1 MiB through the sandboxed runner is
/// capped and reported the same way.
#[test]
fn four_gigabytes_of_stdout_stay_ring_buffered_capped_and_reported() {
    let cap = 64 * 1024;
    let ring = OutputRing::new(cap);
    let total: u64 = 4 * 1024 * 1024 * 1024;
    let chunk = vec![b'x'; 1024 * 1024];
    let mut written: u64 = 0;
    while written + chunk.len() as u64 <= total {
        ring.push(&chunk);
        written += chunk.len() as u64;
    }
    // The sentinel rides the final bytes: only the ring's tail may
    // remember it.
    let sentinel: &[u8] = b"SAYA-TAIL-SENTINEL";
    let mut last = vec![b'y'; 1024 * 1024];
    let tail_len = last.len() - sentinel.len();
    last[tail_len..].copy_from_slice(sentinel);
    ring.push(&last);
    let (text, truncated, dropped, counted) = ring.snapshot();
    assert_eq!(counted, total + last.len() as u64, "every byte was counted");
    assert_eq!(text.len(), cap, "the ring holds the cap and nothing more");
    assert!(truncated, "the truncation must be reported");
    assert_eq!(
        dropped,
        total + last.len() as u64 - cap as u64,
        "the drop is accounted"
    );
    assert!(
        text.ends_with(std::str::from_utf8(sentinel).unwrap()),
        "the ring keeps the tail: the last bytes fed must be the last bytes kept"
    );
}

#[tokio::test]
async fn a_real_flood_is_capped_and_truncation_is_reported() {
    let outcome = standard()
        .tool()
        .run(
            PROGRAM,
            &["flood".to_owned(), "1048576".to_owned()],
            Some(30),
        )
        .await
        .expect("the flood must complete");
    assert_eq!(outcome.exit_code, Some(0));
    assert!(
        outcome.stdout.truncated,
        "1 MiB through a 64 KiB cap must be reported truncated: {outcome:?}"
    );
    assert!(
        outcome.stdout.dropped_bytes >= 1_048_576 - 64 * 1024,
        "the drop is accounted: {outcome:?}"
    );
    assert!(
        outcome.stdout.text.len() <= 64 * 1024,
        "the retained tail is bounded by the cap: {}",
        outcome.stdout.text.len()
    );
    assert!(
        outcome.stdout.text.trim_end().ends_with("flooded"),
        "the ring keeps the tail — the flood's last line survives: {:?}",
        outcome.stdout.text
    );
}

/// The child prints credentials on stdout; both places the capture
/// reaches — the tool's JSON result (the model lane) and the record file
/// it persists into the run workspace (the disk lane) — are scrubbed.
#[tokio::test]
async fn captured_output_is_redacted_in_the_model_result_and_on_disk() {
    let result = credentialed_tool()
        .execute(
            RUN_PROGRAM_TOOL,
            serde_json::json!({
                "program": PROGRAM,
                "args": ["secret"],
                "timeout_seconds": 30
            }),
        )
        .await
        .expect("the call runs");
    let model_text = result["stdout"]["text"]
        .as_str()
        .expect("the model lane's stdout text");
    let record_path = result["record_path"].as_str().expect("the record path");
    let disk_text = fs::read_to_string(record_path).expect("the disk lane must be readable");
    for (lane, text) in [
        ("the model lane", model_text),
        ("the disk lane", disk_text.as_str()),
    ] {
        assert!(
            !text.contains("hunter2"),
            "{lane} must not carry the planted marker secret: {text}"
        );
        assert!(
            !text.contains("sk-live-999"),
            "{lane} must not carry the planted bearer token: {text}"
        );
        assert!(
            text.contains("[redacted]"),
            "{lane} must show the redaction happened: {text}"
        );
    }
}
