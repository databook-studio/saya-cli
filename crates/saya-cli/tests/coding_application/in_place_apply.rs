//! In-place apply: it verifies the reviewed state and records an
//! acknowledgement, never writing to the user's folder.

use super::in_place_review::{create_write, in_place_candidate, records, touch};
use super::support::Fixture;
use saya_cli::CodingError;
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path};

fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(&directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                stack.push(path);
            } else {
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned();
                files.insert(relative, fs::read(&path).unwrap());
            }
        }
    }
    files
}

#[tokio::test]
async fn in_place_apply_is_idempotent() {
    let fixture = Fixture::in_place("in-place-apply-idempotent").await;
    let job = create_write(&fixture, "job-in-place-apply-idempotent", &["tracked.txt"]).await;
    touch(&fixture, job.job_id(), "tracked.txt", Some(b"original\n"));
    fs::write(fixture.repository.join("tracked.txt"), b"changed\n").unwrap();
    let candidate = in_place_candidate(&fixture, &job);
    let review = job.review_existing(&candidate).await.unwrap();
    let first = job.apply_review(&review, true).await.unwrap();
    assert_eq!(first["applied"], json!(true), "{first}");
    assert_eq!(first["mode"], "in_place", "{first}");
    assert_eq!(first["already_applied"], json!(false), "{first}");
    assert_eq!(
        first["source_sha256"],
        json!(review.source_sha256().as_str()),
        "{first}"
    );
    let second = job.apply_review(&review, true).await.unwrap();
    assert_eq!(second["already_applied"], json!(true), "{second}");
    let finalize: Value = serde_json::from_slice(
        &fs::read(records(&fixture, job.job_id()).join("finalize.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(finalize["mode"], "in_place", "{finalize}");
    assert_eq!(
        finalize["source_sha256"],
        json!(review.source_sha256().as_str()),
        "{finalize}"
    );
    assert_eq!(
        finalize["changed_paths"],
        json!(["tracked.txt"]),
        "{finalize}"
    );
}

#[tokio::test]
async fn in_place_apply_writes_no_file() {
    let fixture = Fixture::in_place("in-place-apply-no-file").await;
    let job = create_write(&fixture, "job-in-place-apply-no-file", &["tracked.txt"]).await;
    touch(&fixture, job.job_id(), "tracked.txt", Some(b"original\n"));
    fs::write(fixture.repository.join("tracked.txt"), b"changed\n").unwrap();
    let candidate = in_place_candidate(&fixture, &job);
    let review = job.review_existing(&candidate).await.unwrap();
    let before = tree(&fixture.repository);
    job.apply_review(&review, true).await.unwrap();
    assert_eq!(
        tree(&fixture.repository),
        before,
        "apply acknowledges the folder; it never writes to it"
    );
}

#[tokio::test]
async fn in_place_apply_stale_review_refused() {
    let fixture = Fixture::in_place("in-place-apply-stale").await;
    let job = create_write(&fixture, "job-in-place-apply-stale", &["tracked.txt"]).await;
    touch(&fixture, job.job_id(), "tracked.txt", Some(b"original\n"));
    fs::write(fixture.repository.join("tracked.txt"), b"changed\n").unwrap();
    let candidate = in_place_candidate(&fixture, &job);
    let review = job.review_existing(&candidate).await.unwrap();
    fs::write(fixture.repository.join("tracked.txt"), b"changed again\n").unwrap();
    assert!(matches!(
        job.apply_review(&review, true).await,
        Err(CodingError::StaleReview)
    ));
    assert!(
        !records(&fixture, job.job_id())
            .join("finalize.json")
            .exists(),
        "a stale review writes no finalize record"
    );
}
