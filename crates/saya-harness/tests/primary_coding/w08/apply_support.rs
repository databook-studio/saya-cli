//! W08b apply fixture: three managed checkouts of one job — two children and
//! a candidate — each verified with its published baseline snapshot.

use saya_agent::CancellationToken;
use saya_harness::coding::integration::{
    AcceptedDelta, AssignedPaths, ChangeReceipt, CheckoutDelta,
};
use saya_harness::coding::snapshot::CapturedSnapshot;
use saya_harness::coding::worktree::{
    OwnershipInput, VerifiedCheckout, create_managed_checkout, verify_owned_checkout,
};
use saya_types::ManagedCheckoutId;
use std::path::PathBuf;

use super::create_support::CreateCheckoutFixture;

/// One verified managed checkout with its published baseline and root path.
pub(super) struct Part {
    pub(super) verified: VerifiedCheckout,
    pub(super) snapshot: CapturedSnapshot,
    pub(super) root: PathBuf,
}

/// Children `a` and `b` plus candidate `c`: three checkouts of one job.
pub(super) struct Trio {
    pub(super) fixture: CreateCheckoutFixture,
    pub(super) a: Part,
    pub(super) b: Part,
    pub(super) candidate: Part,
}

pub(super) async fn trio(label: &str) -> Trio {
    let fixture = CreateCheckoutFixture::new(label).await;
    let token = CancellationToken::new();
    let mut parts = Vec::new();
    for suffix in ["a", "b", "c"] {
        let id = ManagedCheckoutId::parse(&format!("{label}-{suffix}")).unwrap();
        create_managed_checkout(&fixture.input(&id), &token)
            .await
            .unwrap();
        let input = OwnershipInput {
            store: &fixture.store,
            service: &fixture.service,
            repository: &fixture.repository,
            artifacts: &fixture.artifacts,
            job_id: &fixture.job,
            checkout_id: &id,
        };
        let verified = verify_owned_checkout(&input, &token).await.unwrap();
        let snapshot = fixture
            .artifacts
            .read_snapshot(verified.snapshot_id())
            .unwrap();
        let root = PathBuf::from(fixture.service.managed_path(&id).unwrap().as_str());
        parts.push(Part {
            verified,
            snapshot,
            root,
        });
    }
    let mut parts = parts.into_iter();
    let a = parts.next().unwrap();
    let b = parts.next().unwrap();
    let candidate = parts.next().unwrap();
    Trio {
        fixture,
        a,
        b,
        candidate,
    }
}

pub(super) fn assigned(paths: &[&str]) -> AssignedPaths {
    AssignedPaths::parse(paths.iter().map(|path| (*path).to_string())).unwrap()
}

pub(super) fn accepted_for<'a>(
    part: &'a Part,
    delta: &'a CheckoutDelta,
    receipt: &ChangeReceipt,
) -> AcceptedDelta<'a> {
    AcceptedDelta::new(&part.verified, receipt, delta).unwrap()
}
