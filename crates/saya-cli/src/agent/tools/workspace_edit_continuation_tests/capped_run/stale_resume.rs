use super::*;

/// The resume path inherits the stale-offset refusal: a continuation that
/// appends at a stale offset refuses, reports the current size and digest,
/// and leaves the file byte-identical. The analogous append contract is covered
/// by `an_offset_mismatch_writes_nothing_and_reports_the_size`.
#[tokio::test]
async fn a_stale_resume_offset_refuses_and_preserves_bytes() {
    let sandbox = Sandbox::new("stale-resume");
    sandbox
        .ws
        .write("report.txt", CHUNK_ONE.as_bytes())
        .expect("seed write must succeed");
    let before = fs::read(sandbox.ws_root().join("report.txt")).expect("seed file must exist");
    let tools = sandbox.tools();
    let stale = before.len().saturating_sub(1) as u64;
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "report.txt",
                "offset": stale,
                "chunk": CHUNK_TWO,
                "expected_digest": sandbox.ws.read("report.txt", 0).expect("seed must read").digest,
            }),
        )
        .await
        .expect_err("a stale resume offset must refuse, not append");
    let text = error.to_string();
    assert!(
        text.contains(&CHUNK_ONE.len().to_string()),
        "the refusal must report the current size so the model can resume, got: {text}"
    );
    assert!(
        text.contains("digest"),
        "the refusal must carry the current digest for the resume precondition, got: {text}"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("report.txt")).expect("file must survive"),
        before,
        "a refused resume leaves the file byte-identical"
    );
}
