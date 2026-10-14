use super::create_support::CreateCheckoutFixture;
use saya_agent::CancellationToken;
use saya_harness::{
    coding::{
        snapshot::{SnapshotArtifacts, SnapshotOmissionPolicy, SnapshotSelection},
        worktree::OwnershipInput,
    },
    workspace::Workspace,
};
use saya_types::ManagedCheckoutId;
pub async fn clean_fixture(label: &str) -> CreateCheckoutFixture {
    let mut fixture = CreateCheckoutFixture::new(label).await;
    let token = CancellationToken::new();
    let policy = SnapshotOmissionPolicy::new(&fixture.repository, Vec::new()).unwrap();
    fixture.plan = fixture
        .service
        .plan_snapshot(
            &fixture.repository,
            fixture.plan.snapshot_id().clone(),
            SnapshotSelection::Head,
            policy,
            &token,
        )
        .await
        .unwrap();
    fixture.artifacts =
        SnapshotArtifacts::open(&fixture.temp.path().join("clean-artifacts")).unwrap();
    fixture.captured = fixture
        .service
        .capture_snapshot(
            &fixture.repository,
            &Workspace::open(fixture.repository.root()).unwrap(),
            &fixture.plan,
            &fixture.artifacts,
            &token,
        )
        .await
        .unwrap();
    fixture
}
pub fn ownership<'a>(
    fixture: &'a CreateCheckoutFixture,
    checkout: &'a ManagedCheckoutId,
) -> OwnershipInput<'a> {
    OwnershipInput {
        store: &fixture.store,
        service: &fixture.service,
        repository: &fixture.repository,
        artifacts: &fixture.artifacts,
        job_id: &fixture.job,
        checkout_id: checkout,
    }
}
