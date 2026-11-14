use super::*;
use crate::direct_prepare::fixture::{checkouts_named, prepared_receipts};

/// The marker a receipt's own bytes carry in this scenario. A readiness view
/// that carried any receipt text would carry it.
const RAW_SECRET_SENTINEL: &str = "RAW_SECRET_SENTINEL";

#[tokio::test]
async fn malformed_prepare_receipt_refuses_without_another_candidate_effect() {
    let provider = Provider::direct_prepare_receipt();
    let fixture = direct_fixture("prepare-malformed", &provider).await;
    let job = job(&fixture, "prepare-malformed").await;
    let output = run(&job).await;
    let before = checkouts_named(&fixture, &job, "candidate-").await;
    assert_eq!(before, 1, "the owner prepared one candidate");

    // Rewrite the durable receipt with a reviewed-source digest that no longer
    // matches the checkout, and a marker in its bytes. The receipt can no
    // longer be read, so it is never readiness.
    let digest = final_envelope(&output)["source_generation_sha256"]
        .as_str()
        .unwrap()
        .to_owned();
    let path = receipt_path(&fixture, &job, &digest);
    let mut receipt: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    receipt["reviewed_source_sha256"] = Value::String(RAW_SECRET_SENTINEL.into());
    receipt["candidate_ready"] = Value::Bool(true);
    fs::write(&path, serde_json::to_vec(&receipt).unwrap()).unwrap();

    let status = job.status().await.unwrap();
    assert_eq!(
        status["readiness"]["candidate_ready"], false,
        "a receipt that cannot be read is never readiness: {status}"
    );
    assert!(
        !status.to_string().contains(RAW_SECRET_SENTINEL),
        "the readiness never carries the receipt's own bytes: {status}"
    );

    // A resumed run finds the receipt present and prepares nothing: no further
    // candidate, and the readiness still refuses to report one.
    let (resumed, output) = run_resumed(&fixture, &job).await;
    assert_eq!(
        checkouts_named(&fixture, &job, "candidate-").await,
        before,
        "a malformed receipt creates no further candidate"
    );
    assert_eq!(
        prepared_receipts(&fixture, &job),
        1,
        "the malformed receipt is still the only one"
    );
    assert_eq!(
        final_envelope(&output)["candidate_ready"],
        false,
        "{output:?}"
    );
    assert_eq!(
        resumed.status().await.unwrap()["readiness"]["candidate_ready"],
        false
    );
}
