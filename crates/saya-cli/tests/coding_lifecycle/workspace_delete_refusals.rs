//! Malformed, private and newly generated targets must reach delete-specific
//! policy. Helpers share the root working checkout, so an out-of-`write_paths`
//! target is not refused before owner admission — there is no per-child path
//! grant.
use super::{
    provider::Provider,
    support::Fixture,
    workspace_delete::{assert_no_apply, captured_results},
};
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingJobRequest};
use saya_types::{BudgetDimension, BudgetEffectState, BudgetObservationValue, JobId, JobState};
use std::{fs, path::Path, time::Duration};

// Existing removal tests do not cover native argument admission/private/ignore
// context. Actual child writes create the targets after snapshot omission, so
// a NotFound or early snapshot refusal cannot masquerade as privacy proof.
#[tokio::test]
async fn native_delete_refusals_keep_malformed_private_and_new_ignored_targets() {
    let provider = Provider::script(false, 22);
    let fixture = Fixture::with_endpoint("delete-refusals", Some(&provider.endpoint)).await;
    fs::write(fixture.repository.join(".gitignore"), b"generated.tmp\n").unwrap();
    fs::write(
        fixture.repository.join("untouched.txt"),
        b"outside child grant\n",
    )
    .unwrap();
    let request = CodingJobRequest::new(
        JobId::parse("delete-refusals").unwrap(),
        "Exercise strict deletion refusal paths without removing or applying anything with helpers"
            .into(),
        vec![
            "tracked.txt".into(),
            ".env".into(),
            "generated.tmp".into(),
            "unused.txt".into(),
        ],
    )
    .unwrap()
    .with_tools_allow(Some(vec![
        "helpers".to_owned(),
        "read".to_owned(),
        "write".to_owned(),
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
    let output = tokio::time::timeout(Duration::from_secs(90), job.run(&NoopEventSink))
        .await
        .unwrap()
        .unwrap();
    assert!(!output.truncated);
    let results = captured_results(&provider);
    for id in [
        "delete-missing-digest",
        "delete-null-digest",
        "delete-uppercase",
        "delete-extra-property",
    ] {
        assert!(
            results[id]["error"].is_string(),
            "actual refusal {id}: {}",
            results[id]
        );
        assert!(results[id].get("removed").is_none());
    }
    assert!(results["delete-private-create"]["digest"].is_string());
    assert!(
        results["delete-private"]["error"]
            .as_str()
            .unwrap()
            .contains("private"),
        "existing private target must reach its policy: {}",
        results["delete-private"]
    );
    assert!(results["delete-ignored-create"]["digest"].is_string());
    assert!(
        results["delete-ignored"]["error"]
            .as_str()
            .unwrap()
            .contains("ignored"),
        "newly generated ignored target must reach live policy: {}",
        results["delete-ignored"]
    );
    assert_eq!(results["delete-retained"]["content"], "     1→original\n");
    let checkouts = fixture
        .store
        .list_managed_checkouts(job.job_id(), None, 10)
        .await
        .unwrap();
    assert!(checkouts.next_cursor.is_none());
    // Helpers share the root working checkout W, so the retained targets
    // are read back from W; there is no per-child checkout.
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
    let root = Path::new(working.ownership.managed_path().as_str());
    for (path, bytes) in [
        ("tracked.txt", b"original\n".as_slice()),
        ("untouched.txt", b"outside child grant\n".as_slice()),
        (".env", b"ordinary synthetic private file\n".as_slice()),
        ("generated.tmp", b"generated source\n".as_slice()),
    ] {
        assert_eq!(fs::read(root.join(path)).unwrap(), bytes);
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
    let mut charged = 0;
    for record in reservations.items {
        let settled = record.settlement.as_ref().unwrap();
        assert_eq!(settled.effect_state(), BudgetEffectState::Settled);
        assert!(
            settled
                .observations()
                .iter()
                .all(|item| !matches!(item.value(), BudgetObservationValue::Unknown))
        );
        for charge in record.reservation.charges() {
            assert_ne!(charge.dimension(), BudgetDimension::CommandSlots);
            if charge.dimension() == BudgetDimension::ToolCalls
                && record.reservation.agent_id().is_some()
            {
                charged += 1;
                assert_eq!(charge.reserved().get(), 1);
                assert!(
                    settled
                        .observations()
                        .iter()
                        .any(|item| item.dimension() == BudgetDimension::ToolCalls
                            && item.value() == BudgetObservationValue::Measured(1))
                );
            }
        }
    }
    assert_eq!(
        charged, 10,
        "every child attempt is admitted and charged; a refused deletion is an owner-admitted effect, not a pre-admission one"
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
    assert_eq!(
        fs::read(fixture.repository.join("untouched.txt")).unwrap(),
        b"outside child grant\n"
    );
    assert!(!fixture.repository.join(".env").exists());
    assert!(!fixture.repository.join("generated.tmp").exists());
}
