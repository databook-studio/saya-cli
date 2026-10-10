//! W08 shared Active fixture: one real verified managed checkout, with its
//! published baseline, root path and cancellation token.

use saya_agent::CancellationToken;
use saya_harness::coding::snapshot::CapturedSnapshot;
use saya_harness::coding::worktree::{
    OwnershipInput, VerifiedCheckout, create_managed_checkout, verify_owned_checkout,
};
use saya_types::ManagedCheckoutId;
use std::path::PathBuf;

use super::create_support::CreateCheckoutFixture;

/// One real Active managed checkout, verified, with its published baseline.
pub(super) struct Active {
    pub(super) fixture: CreateCheckoutFixture,
    pub(super) verified: VerifiedCheckout,
    pub(super) snapshot: CapturedSnapshot,
    pub(super) root: PathBuf,
    pub(super) token: CancellationToken,
}

pub(super) async fn active(label: &str, id: &str) -> Active {
    let fixture = CreateCheckoutFixture::new(label).await;
    let checkout = ManagedCheckoutId::parse(id).unwrap();
    let token = CancellationToken::new();
    create_managed_checkout(&fixture.input(&checkout), &token)
        .await
        .unwrap();
    let input = OwnershipInput {
        store: &fixture.store,
        service: &fixture.service,
        repository: &fixture.repository,
        artifacts: &fixture.artifacts,
        job_id: &fixture.job,
        checkout_id: &checkout,
    };
    let verified = verify_owned_checkout(&input, &token).await.unwrap();
    let snapshot = fixture
        .artifacts
        .read_snapshot(verified.snapshot_id())
        .unwrap();
    let root = PathBuf::from(fixture.service.managed_path(&checkout).unwrap().as_str());
    Active {
        fixture,
        verified,
        snapshot,
        root,
        token,
    }
}
