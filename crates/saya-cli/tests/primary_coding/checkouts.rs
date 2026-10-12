use super::support::Fixture;
use saya_agent::CancellationToken;
use saya_cli::{
    CodingAuthorization, CodingCommand, CodingError, CodingJobRequest, NativeCodingJob,
    dispatch_coding,
};
use saya_types::{JobId, ManagedCheckoutId};
use std::sync::Arc;

const NOT_FOUND: &str = "no managed checkout exists under this id for this job";

type Runtime = Arc<NativeCodingJob>;

async fn job_with_checkout(fixture: &Fixture, job: &str, checkout: &str) -> (JobId, Runtime) {
    let request = CodingJobRequest::new(
        JobId::parse(job).unwrap(),
        "inspect only".into(),
        vec![],
        vec![],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let runtime = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    runtime
        .create_checkout(&ManagedCheckoutId::parse(checkout).unwrap())
        .await
        .unwrap();
    (runtime.job_id().clone(), runtime)
}

fn remove_command(id: &str, checkout: &str, approve: bool) -> CodingCommand {
    CodingCommand::RemoveCheckout {
        id: id.into(),
        checkout: checkout.into(),
        approve,
    }
}

fn reconcile_command(id: &str, checkout: &str, approve: bool) -> CodingCommand {
    CodingCommand::ReconcileCheckout {
        id: id.into(),
        checkout: checkout.into(),
        approve,
    }
}

fn retain_command(id: &str, checkout: &str, approve: bool) -> CodingCommand {
    CodingCommand::RetainCheckout {
        id: id.into(),
        checkout: checkout.into(),
        approve,
    }
}

async fn approved_dispatch(fixture: &Fixture, command: CodingCommand) -> serde_json::Value {
    dispatch_coding(&fixture.application, command)
        .await
        .unwrap()
}

async fn listed_checkouts(fixture: &Fixture, job: &JobId) -> Vec<serde_json::Value> {
    let listed = fixture.application.worktrees(job).await.unwrap();
    listed["checkouts"].as_array().unwrap().clone()
}

fn owner_message(refusal: CodingError, message: &str) {
    let CodingError::Owner(source) = &refusal else {
        panic!("unexpected refusal variant: {refusal}");
    };
    assert_eq!(source.to_string(), message);
}

async fn refused_for_all_operations(fixture: &Fixture, id: &str, checkout: &str, message: &str) {
    for command in [
        remove_command(id, checkout, true),
        reconcile_command(id, checkout, true),
        retain_command(id, checkout, true),
    ] {
        let Err(refusal) = dispatch_coding(&fixture.application, command).await else {
            panic!("the checkout operation must be refused");
        };
        owner_message(refusal, message);
    }
}

async fn refuse_unknown_checkout(runtime: &NativeCodingJob, checkout: &str) {
    let unknown = ManagedCheckoutId::parse(checkout).unwrap();
    for attempt in [
        runtime.remove_checkout(&unknown).await.map(|_| ()),
        runtime.retain_checkout(&unknown).await,
    ] {
        let Err(refusal) = attempt else {
            panic!("an unknown checkout must be refused");
        };
        owner_message(refusal, NOT_FOUND);
    }
    let Err(refusal) = runtime.reconcile_checkout(&unknown).await else {
        panic!("an unknown checkout must be refused");
    };
    owner_message(refusal, "the managed checkout is owned by another job");
}

async fn approved_retain_succeeds(fixture: &Fixture, job: &JobId, checkout: &str) {
    let retained = approved_dispatch(fixture, retain_command(job.as_str(), checkout, true)).await;
    assert_eq!(retained["retained"], checkout);
}

#[tokio::test]
async fn checkout_operations_without_approval_are_refused_and_leave_the_checkout() {
    let fixture = Fixture::new("unapproved-checkouts").await;
    let (job, _) = job_with_checkout(&fixture, "job-unapproved", "guarded-candidate").await;
    let id = job.as_str();
    let checkout = "guarded-candidate";
    for command in [
        remove_command(id, checkout, false),
        reconcile_command(id, checkout, false),
        retain_command(id, checkout, false),
    ] {
        assert!(matches!(
            dispatch_coding(&fixture.application, command).await,
            Err(CodingError::ApprovalRequired)
        ));
    }
    let checkouts = listed_checkouts(&fixture, &job).await;
    assert_eq!(checkouts.len(), 1);
    assert_eq!(checkouts[0]["checkout_id"], checkout);
    assert_eq!(checkouts[0]["lifecycle"], "active");
    approved_retain_succeeds(&fixture, &job, checkout).await;
    let reconciled = approved_dispatch(&fixture, reconcile_command(id, checkout, true)).await;
    assert_eq!(reconciled["reconciliation"], "Unchanged");
    let removed = approved_dispatch(&fixture, remove_command(id, checkout, true)).await;
    assert_eq!(removed["removed"], checkout);
    assert_eq!(
        listed_checkouts(&fixture, &job).await[0]["lifecycle"],
        "removed"
    );
}

#[tokio::test]
async fn malformed_identity_refuses_checkout_operations_and_leaves_the_checkout() {
    let fixture = Fixture::new("malformed-identity").await;
    let (job, _) = job_with_checkout(&fixture, "job-malformed", "checked-candidate").await;
    refused_for_all_operations(
        &fixture,
        "job bad id",
        "checked-candidate",
        "job id must be 1-128 ASCII letters, digits, hyphens, or underscores",
    )
    .await;
    refused_for_all_operations(
        &fixture,
        job.as_str(),
        "not a checkout",
        "managed checkout id must be 1-128 ASCII letters, digits, hyphens, or underscores",
    )
    .await;
    assert_eq!(listed_checkouts(&fixture, &job).await.len(), 1);
    approved_retain_succeeds(&fixture, &job, "checked-candidate").await;
}

#[tokio::test]
async fn unrecorded_job_refuses_checkout_operations_without_effects_until_it_exists() {
    let fixture = Fixture::new("unrecorded-job").await;
    for command in [
        remove_command("job-unrecorded", "lost-candidate", true),
        reconcile_command("job-unrecorded", "lost-candidate", true),
        retain_command("job-unrecorded", "lost-candidate", true),
    ] {
        assert!(matches!(
            dispatch_coding(&fixture.application, command).await,
            Err(CodingError::NotOwned)
        ));
    }
    job_with_checkout(&fixture, "job-unrecorded", "recovered-candidate").await;
    let reconciled = approved_dispatch(
        &fixture,
        reconcile_command("job-unrecorded", "recovered-candidate", true),
    )
    .await;
    assert_eq!(reconciled["reconciliation"], "Unchanged");
}

#[tokio::test]
async fn unknown_checkout_id_is_refused_by_every_operation_until_it_exists() {
    let fixture = Fixture::new("unknown-checkout").await;
    let (job, runtime) =
        job_with_checkout(&fixture, "job-unknown-checkout", "real-candidate").await;
    refuse_unknown_checkout(&runtime, "never-created").await;
    assert_eq!(listed_checkouts(&fixture, &job).await.len(), 1);
    let unknown = ManagedCheckoutId::parse("never-created").unwrap();
    runtime.create_checkout(&unknown).await.unwrap();
    runtime.retain_checkout(&unknown).await.unwrap();
    let reconciled = runtime.reconcile_checkout(&unknown).await.unwrap();
    assert_eq!(reconciled["reconciliation"], "Unchanged");
    runtime.remove_checkout(&unknown).await.unwrap();
}

#[tokio::test]
async fn remove_checkout_with_uncommitted_work_is_refused_until_the_work_is_reverted() {
    let fixture = Fixture::new("remove-work").await;
    let (job, runtime) = job_with_checkout(&fixture, "job-remove-work", "work-candidate").await;
    let candidate = ManagedCheckoutId::parse("work-candidate").unwrap();
    let record = fixture
        .store
        .get_managed_checkout(&job, &candidate)
        .await
        .unwrap()
        .unwrap();
    let tracked =
        std::path::Path::new(record.ownership.managed_path().as_str()).join("tracked.txt");
    std::fs::write(&tracked, b"candidate edit\n").unwrap();
    let Err(refusal) = runtime.remove_checkout(&candidate).await else {
        panic!("uncommitted work must be retained, not removed");
    };
    owner_message(refusal, "checkout contains work that must be retained");
    assert_eq!(std::fs::read(&tracked).unwrap(), b"candidate edit\n");
    std::fs::write(&tracked, b"original\n").unwrap();
    runtime.remove_checkout(&candidate).await.unwrap();
    assert_eq!(
        listed_checkouts(&fixture, &job).await[0]["lifecycle"],
        "removed"
    );
}

#[tokio::test]
async fn foreign_checkout_is_refused_while_the_own_checkout_stays_operable() {
    let fixture = Fixture::new("foreign-checkout").await;
    let (_, owner) = job_with_checkout(&fixture, "job-owner", "shared-candidate").await;
    let (foreign_id, foreign) = job_with_checkout(&fixture, "job-foreign", "own-candidate").await;
    let shared = ManagedCheckoutId::parse("shared-candidate").unwrap();
    let Err(refusal) = foreign.reconcile_checkout(&shared).await else {
        panic!("a foreign checkout must not be reconciled");
    };
    owner_message(refusal, "the managed checkout is owned by another job");
    assert_eq!(listed_checkouts(&fixture, owner.job_id()).await.len(), 1);
    assert!(listed_checkouts(&fixture, &foreign_id).await.len() == 1);
    let own = ManagedCheckoutId::parse("own-candidate").unwrap();
    let reconciled = foreign.reconcile_checkout(&own).await.unwrap();
    assert_eq!(reconciled["reconciliation"], "Unchanged");
}
