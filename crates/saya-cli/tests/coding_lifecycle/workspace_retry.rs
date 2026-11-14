//! Missing reads become stale only after a real settled same-path mutation.
use super::{provider::Provider, support::Fixture};
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingJobRequest};
use saya_types::{BudgetDimension, BudgetEffectState, JobId, JobState};
use serde_json::Value;
use std::{collections::BTreeMap, fs, time::Duration};

#[tokio::test]
async fn a_missing_read_recovers_after_a_real_owned_creation() {
    let provider = Provider::workspace_retry();
    let fixture = Fixture::with_endpoint("workspace-retry", Some(&provider.endpoint)).await;
    let request = CodingJobRequest::new(
        JobId::parse("workspace-retry").unwrap(),
        "Create and reread the two owned files, then review their candidate with helpers".into(),
        vec!["new-a.txt".into(), "new-b.txt".into()],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    let output = tokio::time::timeout(Duration::from_secs(40), job.run(&NoopEventSink))
        .await
        .unwrap()
        .unwrap();
    assert!(!output.truncated);
    let results = captured_results(&provider);
    for actor in ["child-a", "child-b"] {
        let missing = &results[&format!("missing-{actor}")];
        assert!(
            missing["error"].is_string(),
            "real initial absence must fail"
        );
        let created = &results[&format!("create-{actor}")];
        assert!(
            created["digest"].is_string(),
            "actual creation must publish identity"
        );
        let reread = &results[&format!("reread-{actor}")];
        assert_eq!(
            reread["content"],
            format!("     1→created {actor}\n"),
            "same-path read must execute after creation: {reread}"
        );
        assert_eq!(reread["digest"], created["digest"]);
        assert!(reread.get("error").is_none());
    }
    let children = fixture
        .store
        .list_coding_children(job.job_id(), None, 10)
        .await
        .unwrap();
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
    assert!(reservations.items.iter().all(|record| {
        record
            .settlement
            .as_ref()
            .is_some_and(|settlement| settlement.effect_state() == BudgetEffectState::Settled)
    }));
    assert_eq!(
        reservations
            .items
            .iter()
            .filter(|record| record.reservation.agent_id().is_some()
                && record
                    .reservation
                    .charges()
                    .iter()
                    .any(|charge| charge.dimension() == BudgetDimension::ToolCalls))
            .count(),
        6,
        "missing and recovered reads remain charged"
    );
    // Direct mode has no root `coding_review`: the owner preparation
    // assembles the one candidate from the root working checkout W (both
    // finished children are merged into it). The prepared receipt's field set
    // stays closed (`prepare_receipt.rs`), so the per-path change list is
    // reported by the direct advisory readiness instead — the run's own final
    // envelope carries it, so no status poll is needed.
    let envelope: Value =
        serde_json::from_str(&output.answer).expect("the run's answer is the readiness envelope");
    let changed = envelope["readiness"]["changed_paths"]
        .as_array()
        .unwrap_or_else(|| panic!("the readiness lists the changed paths: {envelope}"));
    assert_eq!(
        changed.len(),
        2,
        "both real creations changed W: {envelope}"
    );
    for path in ["new-a.txt", "new-b.txt"] {
        assert!(
            changed
                .iter()
                .any(|change| change["path"] == path && change["kind"] == "created"),
            "the candidate's creation of {path} is listed: {envelope}"
        );
    }
    // The retired `coding_review` has no replacement: the owner prepares at the
    // end of the run, and the run's own final envelope names the prepared
    // candidate and its source generation.
    let candidate = envelope["readiness"]["candidate"]
        .as_str()
        .unwrap_or_else(|| panic!("the owner prepared a candidate: {envelope}"));
    let candidate_root = fixture
        .store
        .list_managed_checkouts(job.job_id(), None, 10)
        .await
        .unwrap()
        .items
        .into_iter()
        .find(|record| record.ownership.managed_checkout_id().as_str() == candidate)
        .map(|record| std::path::PathBuf::from(record.ownership.managed_path().as_str()))
        .expect("the prepared candidate is actually owned and stored");
    for path in ["new-a.txt", "new-b.txt"] {
        assert!(
            candidate_root.join(path).is_file(),
            "the candidate carries the real creation {path}: {envelope}"
        );
        assert!(!fixture.repository.join(path).exists());
    }
    assert_eq!(
        fs::read(fixture.repository.join("tracked.txt")).unwrap(),
        b"original\n"
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
}

fn captured_results(provider: &Provider) -> BTreeMap<String, Value> {
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
                    if !["missing-", "create-", "reread-"]
                        .iter()
                        .any(|prefix| id.starts_with(prefix))
                    {
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
