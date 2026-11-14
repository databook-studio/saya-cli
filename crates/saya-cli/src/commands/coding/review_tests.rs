//! The operator's review and apply is the check: no check receipt is
//! required.

#![cfg(unix)]

use super::super::direct_root_fixture::fixture;
use saya_types::ManagedCheckoutId;

#[tokio::test]
async fn review_and_apply_succeed_without_any_check_receipt() {
    let f = fixture("review-no-checks", vec!["tracked.txt".into()]).await;
    let candidate = ManagedCheckoutId::parse("review-no-checks-candidate").unwrap();
    f.job.create_checkout(&candidate).await.unwrap();
    let record = f
        .job
        .application
        .store
        .get_managed_checkout(f.job.job_id(), &candidate)
        .await
        .unwrap()
        .unwrap();
    std::fs::write(
        std::path::Path::new(record.ownership.managed_path().as_str()).join("tracked.txt"),
        b"candidate change\n",
    )
    .unwrap();

    let review = f.job.review_existing(&candidate).await.unwrap();
    assert!(
        review
            .preview()
            .get("checks")
            .and_then(serde_json::Value::as_array)
            .is_none_or(Vec::is_empty),
        "no check receipt is recorded: {}",
        review.preview()
    );

    f.job
        .apply_review(&review, true)
        .await
        .expect("apply needs no successful check receipt");
    assert_eq!(
        std::fs::read(f.root.join("repo").join("tracked.txt")).unwrap(),
        b"candidate change\n",
        "the reviewed candidate reached the original checkout"
    );
}
