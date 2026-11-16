use super::*;
use crate::direct_prepare::fixture::{checkouts_named, prepared_receipts, review_lines};

#[tokio::test]
async fn prepare_candidate_replays_exact_receipts_and_follows_the_repair() {
    // Mode 90: the root writes a broken source into W and then repairs it, so
    // the owner's single preparation is bound to the repaired generation.
    let provider = Provider::direct_prepare_repair();
    let fixture = direct_fixture("prepare-candidate", &provider).await;
    let job = job(&fixture, "prepare-candidate").await;
    let output = run(&job).await;
    let envelope = final_envelope(&output);
    assert_eq!(envelope["candidate_ready"], true, "{envelope}");
    let candidate = envelope["candidate"].as_str().unwrap().to_owned();
    let digest = envelope["source_generation_sha256"]
        .as_str()
        .unwrap()
        .to_owned();

    // The durable receipt carries no check at all and says the candidate is
    // ready: the operator's review and apply is the check.
    let receipt: Value =
        serde_json::from_slice(&fs::read(receipt_path(&fixture, &job, &digest)).unwrap()).unwrap();
    assert_eq!(receipt["checks"], serde_json::json!([]), "{receipt}");
    assert_eq!(receipt["candidate_ready"], true, "{receipt}");
    assert_eq!(receipt["candidate"], candidate, "{receipt}");

    // The candidate is bound to the REPAIRED source, not the broken one.
    let checkout = checkout_path(&fixture, &job, &candidate).await;
    assert_eq!(
        fs::read_to_string(checkout.join("tracked.txt")).unwrap(),
        "FIXED\n",
        "the candidate carries the repair"
    );
    assert_eq!(
        fs::read(fixture.repository.join("tracked.txt")).unwrap(),
        b"original\n",
        "preparation never changes the original tree"
    );
    assert_eq!(checkouts_named(&fixture, &job, "candidate-").await, 1);
    assert_eq!(prepared_receipts(&fixture, &job), 1);
    assert_eq!(review_lines(&fixture, &job), 1);

    // A resume replays the same receipt exactly: no new checkout, no new
    // review, and the readiness reports the same candidate.
    let (resumed, output) = run_resumed(&fixture, &job).await;
    let replay = final_envelope(&output);
    assert_eq!(replay["candidate"], envelope["candidate"], "{replay}");
    assert_eq!(
        replay["source_generation_sha256"], envelope["source_generation_sha256"],
        "{replay}"
    );
    assert_eq!(checkouts_named(&fixture, &job, "candidate-").await, 1);
    assert_eq!(prepared_receipts(&fixture, &job), 1);
    assert_eq!(review_lines(&fixture, &job), 1);
    let status = resumed.status().await.unwrap();
    assert_eq!(status["readiness"]["state"], "candidate_ready", "{status}");
    assert_eq!(
        status["readiness"]["candidate"], envelope["candidate"],
        "{status}"
    );
}
