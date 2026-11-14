//! Merge fixture: one job with two managed checkouts of the same published
//! baseline — the root's agent-less working checkout W and one child checkout
//! — plus an agent-owned checkout for the ownership-refusal proof. Reuses the
//! create fixture linked by the target root.

use saya_agent::CancellationToken;
use saya_harness::coding::integration::{
    AcceptedDelta, AssignedPaths, ChangeReceipt, CheckoutDelta,
};
use saya_harness::coding::snapshot::CapturedSnapshot;
use saya_harness::coding::worktree::{
    OwnershipInput, VerifiedCheckout, create_managed_checkout, verify_owned_checkout,
};
use saya_store::NewCodingChild;
use saya_types::{AgentId, InvocationId, ManagedCheckoutId};
use std::path::PathBuf;

use super::create_support::CreateCheckoutFixture;

/// One verified managed checkout with its published baseline and root path.
pub(super) struct Part {
    pub(super) verified: VerifiedCheckout,
    pub(super) snapshot: CapturedSnapshot,
    pub(super) root: PathBuf,
}

/// Working checkout W and one child checkout of one job, both verified
/// against the same captured baseline.
pub(super) struct Pair {
    pub(super) fixture: CreateCheckoutFixture,
    pub(super) working: Part,
    pub(super) child: Part,
}

/// An agent-owned checkout and its fixture, for the ownership-refusal proof.
pub(super) struct OwnedTarget {
    pub(super) fixture: CreateCheckoutFixture,
    pub(super) checkout: Part,
}

/// Builds W and the child: two agent-less checkouts of one job, materialized
/// from one plan and verified against its captured baseline.
pub(super) async fn pair(label: &str) -> Pair {
    let fixture = CreateCheckoutFixture::new(label).await;
    let working = verified_part(&fixture, &format!("{label}-w")).await;
    let child = verified_part(&fixture, &format!("{label}-child")).await;
    Pair {
        fixture,
        working,
        child,
    }
}

/// Builds one agent-owned checkout: the child row the store requires, then
/// the checkout created for that agent and verified at use time.
pub(super) async fn owned_checkout(label: &str, agent: &str) -> OwnedTarget {
    let fixture = CreateCheckoutFixture::new(label).await;
    let checkout_id = ManagedCheckoutId::parse(label).unwrap();
    let agent_id = AgentId::parse(agent).unwrap();
    fixture
        .store
        .create_coding_child(NewCodingChild {
            job_id: fixture.job.clone(),
            agent_id: agent_id.clone(),
            current_invocation_id: InvocationId::parse(&format!("{agent}-invocation")).unwrap(),
        })
        .await
        .unwrap();
    let mut input = fixture.input(&checkout_id);
    input.agent_id = Some(agent_id);
    let token = CancellationToken::new();
    create_managed_checkout(&input, &token).await.unwrap();
    let checkout = verify_part(&fixture, &checkout_id, &token).await;
    OwnedTarget { fixture, checkout }
}

/// Creates one agent-less checkout of the fixture's job, verifies it, and
/// reads back its published baseline snapshot.
async fn verified_part(fixture: &CreateCheckoutFixture, id: &str) -> Part {
    let checkout_id = ManagedCheckoutId::parse(id).unwrap();
    let token = CancellationToken::new();
    create_managed_checkout(&fixture.input(&checkout_id), &token)
        .await
        .unwrap();
    verify_part(fixture, &checkout_id, &token).await
}

/// Verifies one checkout at use time and reads back its published baseline.
async fn verify_part(
    fixture: &CreateCheckoutFixture,
    checkout_id: &ManagedCheckoutId,
    token: &CancellationToken,
) -> Part {
    let input = OwnershipInput {
        store: &fixture.store,
        service: &fixture.service,
        repository: &fixture.repository,
        artifacts: &fixture.artifacts,
        job_id: &fixture.job,
        checkout_id,
    };
    let verified = verify_owned_checkout(&input, token).await.unwrap();
    let snapshot = fixture
        .artifacts
        .read_snapshot(verified.snapshot_id())
        .unwrap();
    let root = PathBuf::from(fixture.service.managed_path(checkout_id).unwrap().as_str());
    Part {
        verified,
        snapshot,
        root,
    }
}

/// The assigned-path set for one receipt.
pub(super) fn assigned(paths: &[&str]) -> AssignedPaths {
    AssignedPaths::parse(paths.iter().map(|path| (*path).to_string())).unwrap()
}

/// Fails closed unless the receipt describes `delta` exactly.
pub(super) fn accepted_for<'a>(
    part: &'a Part,
    delta: &'a CheckoutDelta,
    receipt: &ChangeReceipt,
) -> AcceptedDelta<'a> {
    AcceptedDelta::new(&part.verified, receipt, delta).unwrap()
}
