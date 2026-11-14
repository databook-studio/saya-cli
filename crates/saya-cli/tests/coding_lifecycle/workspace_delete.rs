//! Native deletion uses the real read identity and exact child authority.
use super::{provider::Provider, support::Fixture};
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingJobRequest};
use saya_types::{
    BudgetDimension, BudgetEffectState, BudgetObservationValue, CodingEventKind, JobId, JobState,
    RepositoryMutationAction,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::Path, time::Duration};

// Protects actual model-tool -> exact child admission -> contained removal ->
// candidate delta and authoritative settlement. Current public removal tests
// never cross the native tool boundary; a string-named future call compiles on
// baseline and fails because the actual owned file remains. Provider reads
// the digest only from its real tool message, never from a fixture constant.
#[tokio::test]
async fn owned_file_deletion_uses_observed_digest_and_preserves_original() {
    let provider = Provider::script(false, 20);
    let fixture = Fixture::with_endpoint("native-delete", Some(&provider.endpoint)).await;
    let request = CodingJobRequest::new(
        JobId::parse("native-delete").unwrap(),
        "Delete one obsolete owned file using its actual observed digest and review the candidate with helpers"
            .into(),
        vec!["tracked.txt".into(), "unused.txt".into()],
    )
    .unwrap()
    .with_tools_allow(Some(vec![
        "helpers".to_owned(),
        "read".to_owned(),
        "delete".to_owned(),
    ]))
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    let output = tokio::time::timeout(Duration::from_secs(60), job.run(&NoopEventSink))
        .await
        .unwrap()
        .unwrap();
    assert!(!output.truncated);
    let results = captured_results(&provider);
    let observed = &results["delete-observed"];
    assert_eq!(
        observed["content"], "     1→original\n",
        "the actual owned read must execute"
    );
    let digest = observed["digest"]
        .as_str()
        .expect("actual owner read digest");
    assert_eq!(digest, format!("{:x}", Sha256::digest(b"original\n")));
    assert_ne!(digest, "0".repeat(64));
    let after_stale = &results["delete-after-stale"];
    assert_eq!(
        after_stale["content"], "     1→original\n",
        "stale removal must retain actual bytes"
    );
    assert_eq!(after_stale["digest"], digest);
    assert!(results["delete-stale"]["error"].is_string());
    assert!(results["delete-stale"].get("removed").is_none());
    assert_eq!(
        fs::read(fixture.repository.join("tracked.txt")).unwrap(),
        b"original\n"
    );
    let checkouts = fixture
        .store
        .list_managed_checkouts(job.job_id(), None, 10)
        .await
        .unwrap();
    assert!(checkouts.next_cursor.is_none());
    // Helpers share the root working checkout W: the child's confirmed
    // deletion is W's, and there is no per-child checkout to read it from.
    let working = checkouts
        .items
        .iter()
        .find(|checkout| {
            checkout
                .ownership
                .managed_checkout_id()
                .as_str()
                .starts_with("work-")
        })
        .expect("the root working checkout W exists");
    let working_root = Path::new(working.ownership.managed_path().as_str());
    assert!(
        !working_root.join("tracked.txt").exists(),
        "correct-digest native deletion did not remove the owned file; actual tool result: {}",
        results["delete-confirmed"]
    );
    let removed = &results["delete-confirmed"];
    assert_eq!(removed["path"], "tracked.txt");
    assert_eq!(removed["removed"], true);
    assert_eq!(removed["removed_digest"], digest);
    assert!(
        removed.get("digest").is_none(),
        "deletion has no current file digest"
    );
    assert!(
        removed.get("size").is_none(),
        "deletion is not an empty-file observation"
    );
    let after_delete = &results["delete-after-confirmed"];
    let missing = after_delete["error"]
        .as_str()
        .expect("an actual post-delete read must fail");
    assert!(
        missing.contains("does not exist"),
        "actual absence expected: {missing}"
    );
    assert!(
        !missing.contains("already failed"),
        "deletion cannot invent a retry-cache result"
    );
    // Direct mode has no root `coding_review`: the owner preparation
    // assembles the one candidate from the root working checkout W (the
    // finished child's deletion is merged into it). The prepared receipt's
    // field set stays closed (`prepare_receipt.rs`), so the per-path change
    // list is reported by the direct advisory readiness instead — the run's
    // own final envelope carries it, so no status poll is needed.
    let envelope: Value =
        serde_json::from_str(&output.answer).expect("the run's answer is the readiness envelope");
    let changed = envelope["readiness"]["changed_paths"]
        .as_array()
        .unwrap_or_else(|| panic!("the readiness lists the changed paths: {envelope}"));
    assert_eq!(
        changed.len(),
        1,
        "exactly the confirmed deletion changed W: {envelope}"
    );
    assert_eq!(changed[0]["path"], "tracked.txt", "{envelope}");
    assert_eq!(changed[0]["kind"], "deleted", "{envelope}");
    assert_eq!(
        envelope["readiness"]["changed_paths_truncated"], false,
        "{envelope}"
    );
    // The retired `coding_review` has no replacement: the owner prepares at the
    // end of the run, and the run's own final envelope names the prepared
    // candidate.
    let candidate_id = envelope["readiness"]["candidate"]
        .as_str()
        .unwrap_or_else(|| panic!("the owner prepared a candidate: {envelope}"));
    let candidate = checkouts
        .items
        .iter()
        .find(|checkout| checkout.ownership.managed_checkout_id().as_str() == candidate_id)
        .expect("the deletion candidate is actually owned and stored");
    assert!(
        !Path::new(candidate.ownership.managed_path().as_str())
            .join("tracked.txt")
            .exists(),
        "the assembled candidate carries the confirmed deletion: {envelope}"
    );
    let children = fixture
        .store
        .list_coding_children(job.job_id(), None, 10)
        .await
        .unwrap();
    assert!(children.next_cursor.is_none());
    assert_eq!(children.items.len(), 2);
    assert!(
        children
            .items
            .iter()
            .all(|child| child.state == JobState::Completed)
    );
    let reservations = fixture
        .store
        .list_coding_budget_reservations(job.job_id(), None, 100)
        .await
        .unwrap();
    assert!(reservations.next_cursor.is_none());
    let mut child_tools = 0;
    for record in reservations.items {
        let settlement = record
            .settlement
            .as_ref()
            .expect("all actual effects settled");
        assert_eq!(settlement.effect_state(), BudgetEffectState::Settled);
        assert!(settlement.observations().iter().all(|observation| {
            !matches!(observation.value(), BudgetObservationValue::Unknown)
        }));
        for charge in record.reservation.charges() {
            assert_ne!(
                charge.dimension(),
                BudgetDimension::CommandSlots,
                "no process performed deletion"
            );
            if charge.dimension() == BudgetDimension::ToolCalls
                && record.reservation.agent_id().is_some()
            {
                child_tools += 1;
                assert_eq!(record.reservation.agent_id().unwrap().as_str(), "child-a");
                assert_eq!(charge.reserved().get(), 1);
                assert!(settlement.observations().iter().any(|observation| {
                    observation.dimension() == BudgetDimension::ToolCalls
                        && observation.value() == BudgetObservationValue::Measured(1)
                }));
            }
        }
    }
    assert_eq!(
        child_tools, 5,
        "real reads, stale refusal and confirmed delete remain charged"
    );
    assert_eq!(
        fixture
            .store
            .coding_recovery_status(job.job_id())
            .await
            .unwrap()
            .unresolved_reservations,
        0
    );
    assert_no_apply(&fixture, job.job_id()).await;
    assert_eq!(
        fs::read(fixture.repository.join("tracked.txt")).unwrap(),
        b"original\n"
    );
    assert!(!fixture.repository.join("unused.txt").exists());
}

pub(super) fn captured_results(provider: &Provider) -> BTreeMap<String, Value> {
    provider
        .bodies
        .lock()
        .unwrap()
        .iter()
        .flat_map(|body| {
            let body: Value = serde_json::from_str(body).unwrap();
            body["messages"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|message| {
                    let id = message["tool_call_id"].as_str()?;
                    if !id.starts_with("delete-") {
                        return None;
                    }
                    Some((
                        id.to_owned(),
                        serde_json::from_str(message["content"].as_str()?).unwrap(),
                    ))
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

pub(super) async fn assert_no_apply(fixture: &Fixture, job: &JobId) {
    let mut cursor = None;
    for _ in 0..32 {
        let page = fixture
            .store
            .list_coding_events(job, cursor, saya_store::MAX_CODING_EVENT_PAGE_SIZE)
            .await
            .unwrap();
        assert!(
            page.events.iter().all(|event| {
                !matches!(event.event(), CodingEventKind::CheckoutOperationIntent { operation }
                if operation.action() == RepositoryMutationAction::ApplyBack)
            }),
            "delete and review must not apply to the original"
        );
        let Some(next) = page.next_cursor else { return };
        assert!(cursor.is_none_or(|previous| next > previous));
        cursor = Some(next);
    }
    panic!("native deletion fixture exceeded its bounded full event scan");
}
