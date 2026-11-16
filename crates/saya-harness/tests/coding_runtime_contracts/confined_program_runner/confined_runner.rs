//! Confined-runner battery: the proven confinement's `PlatformSupport`
//! bridge.

use std::path::Path;

use saya_agent::CancellationToken;
use saya_harness::coding::policy::PlatformSupport;
use saya_harness::coding::worktree::{
    OwnershipInput, ScratchScope, confine_checkout, create_managed_checkout, verify_owned_checkout,
};
use saya_types::{JobId, ManagedCheckoutId};

use super::create_support::CreateCheckoutFixture;

/// The checkout under confinement in every test.
const A_ID: &str = "checkout-r05b2-a";

/// One ownership input over the fixture for the given asking job.
fn ownership_input<'a>(
    fixture: &'a CreateCheckoutFixture,
    job_id: &'a JobId,
    checkout_id: &'a ManagedCheckoutId,
) -> OwnershipInput<'a> {
    OwnershipInput {
        store: &fixture.store,
        service: &fixture.service,
        repository: &fixture.repository,
        artifacts: &fixture.artifacts,
        job_id,
        checkout_id,
    }
}

/// The fixture-to-proof flow every test walks: create, verify, confine to /bin.
macro_rules! confined {
    ($fixture:ident, $a:ident, $token:ident) => {{
        create_managed_checkout(&$fixture.input(&$a), &$token)
            .await
            .unwrap();
        let input = ownership_input(&$fixture, &$fixture.job, &$a);
        let verified = verify_owned_checkout(&input, &$token).await.unwrap();
        confine_checkout(&verified, Path::new("/bin"), ScratchScope::Root).unwrap()
    }};
}

/// A proven confinement is itself the evidence: it maps to the trusted host
/// support — edits in exactly the owned checkout, no network, no database
/// reads.
#[tokio::test]
async fn proven_confinement_maps_to_the_trusted_host_support() {
    let fixture = CreateCheckoutFixture::new("r05b2-bridge").await;
    let a = ManagedCheckoutId::parse(A_ID).unwrap();
    let token = CancellationToken::new();
    let confined = confined!(fixture, a, token);

    let platform: PlatformSupport = confined.platform_support();
    assert!(
        platform.workspace_edits,
        "the single writable root is the owned checkout"
    );
    assert!(!platform.network_fetch, "the net is empty by construction");
    assert!(!platform.database_reads);
}
