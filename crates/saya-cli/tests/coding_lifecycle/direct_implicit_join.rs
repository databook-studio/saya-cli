//! Direct-mode implicit join: a root that delegated and never awaited still
//! ends with its children's work in the candidate. When the root loop ends
//! while a schedule is live, the owner settles it; every helper works in the
//! root working checkout W directly, so the child's work is already there,
//! and only then does the owner preparation run — so the candidate carries the
//! child's delta. The root's own change and the child's are both in W.

use super::direct_prepare::fixture::{checkout_path, records_dir, run_direct_prepare_output};
use super::direct_task::root_checkout;
use serde_json::Value;

/// The final result a write-path run ends with: the owner's readiness envelope.
fn final_envelope(output: &saya_agent::AgentOutput) -> Value {
    serde_json::from_str(&output.answer).expect("the run's answer is the readiness envelope")
}

#[tokio::test]
async fn a_job_that_delegates_without_awaiting_still_ends_with_its_childs_work_in_the_candidate() {
    // Mode 79: the root writes `root-edit.txt` in W, delegates one child it
    // never awaits, then answers. The implicit join settles the child — whose
    // write is already in W — and the owner preparation then runs as at the end
    // of any run, so the job ends with its child's work in the prepared
    // candidate.
    let (fixture, job, _provider, output) = run_direct_prepare_output(
        "direct-finish-implicit-join",
        79,
        vec!["root-edit.txt".into(), "child-file.txt".into()],
        vec!["grep".into()],
    )
    .await;
    let working = root_checkout(&fixture, &job).await;
    assert_eq!(
        std::fs::read_to_string(working.join("child-file.txt")).unwrap(),
        "child wrote\n",
        "the never-awaited child's work is in W"
    );
    assert_eq!(
        std::fs::read_to_string(working.join("root-edit.txt")).unwrap(),
        "root wrote\n",
        "the root's own change is in W too"
    );
    let envelope = final_envelope(&output);
    assert_eq!(envelope["candidate_ready"], true, "{envelope}");
    assert_eq!(envelope["w_changed"], true, "{envelope}");
    let digest = envelope["source_generation_sha256"]
        .as_str()
        .unwrap_or_else(|| panic!("the owner prepared a candidate: {envelope}"));
    assert!(
        records_dir(&fixture, &job)
            .join(format!("prepared-candidate-{digest}.json"))
            .exists(),
        "the owner's receipt is durable for the current generation"
    );
    let candidate = checkout_path(&fixture, &job, envelope["candidate"].as_str().unwrap()).await;
    assert_eq!(
        std::fs::read_to_string(candidate.join("child-file.txt")).unwrap(),
        "child wrote\n",
        "the prepared candidate carries the child's work"
    );
    assert!(job.failure().is_none(), "preparation is not a root failure");
}
