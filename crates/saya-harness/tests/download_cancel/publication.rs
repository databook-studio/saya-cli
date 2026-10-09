use super::transports::*;
use super::*;

/// The body's end-of-stream call cancels the turn's token: the download
/// stops right before the promotion. Nothing is published; the complete
/// partial and its matching sidecar stay for an explicit retry.
#[tokio::test]
async fn cancelling_after_the_last_byte_but_before_publication_publishes_nothing() {
    let content: Vec<u8> = (0..40u8).collect();
    let (root, workspace) = workspace("before-publish");
    let budget = DownloadBudget::default();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let token = CancellationToken::new();
    let net = EndCancelNet {
        content: Arc::new(content.clone()),
        chunk: 8,
        cancellation: token.clone(),
        calls: Arc::clone(&calls),
    };
    let task = spawn_download(net, workspace, budget, token.clone());
    let error = tokio::time::timeout(ABORT, task)
        .await
        .expect("the download must not hang")
        .expect("the download task joins")
        .expect_err("cancelled before publication, so an error");
    assert!(matches!(error, DownloadError::Cancelled), "{error:?}");
    assert!(
        !root.join(DEST).exists(),
        "nothing was published: the destination is absent"
    );
    assert_eq!(
        std::fs::read(root.join(PART)).expect("the complete partial stays"),
        content,
        "the complete partial is left for an explicit retry"
    );
    let sidecar = sidecar_json(&root);
    assert_eq!(sidecar["len"], content.len() as u64);
    assert_eq!(sidecar["sha256"], digest(&content));
}
