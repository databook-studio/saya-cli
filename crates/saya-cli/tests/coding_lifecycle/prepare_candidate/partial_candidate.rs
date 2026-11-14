use super::*;
use crate::direct_prepare::fixture::{checkouts_named, prepared_receipts};

#[tokio::test]
async fn partial_candidate_without_receipt_refuses_before_a_new_checkout() {
    let provider = Provider::direct_prepare_receipt();
    let fixture = direct_fixture("prepare-partial", &provider).await;
    let job = job(&fixture, "prepare-partial").await;
    let output = run(&job).await;
    let before = checkouts_named(&fixture, &job, "candidate-").await;
    assert_eq!(before, 1, "the owner prepared one candidate");

    // Remove the receipt: the generation has a candidate checkout and no
    // receipt, which is an incomplete candidate state. The resumed owner tries
    // to prepare and is refused before it creates another checkout.
    let digest = final_envelope(&output)["source_generation_sha256"]
        .as_str()
        .unwrap()
        .to_owned();
    fs::remove_file(receipt_path(&fixture, &job, &digest)).unwrap();

    let (resumed, output) = run_resumed(&fixture, &job).await;
    assert_eq!(
        checkouts_named(&fixture, &job, "candidate-").await,
        before,
        "a partial candidate is refused before another checkout"
    );
    assert_eq!(
        prepared_receipts(&fixture, &job),
        0,
        "the refused preparation wrote no receipt"
    );
    let envelope = final_envelope(&output);
    assert_eq!(envelope["candidate_ready"], false, "{envelope}");
    assert_eq!(
        envelope["readiness"]["preparation"]["reason"], "candidate_state",
        "the not-ready view names the closed reason: {envelope}"
    );
    assert_eq!(
        resumed.status().await.unwrap()["readiness"]["candidate_ready"],
        false
    );
}
