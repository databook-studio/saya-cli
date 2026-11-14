use saya_harness::coding::policy::{
    AuthorityRefusal, ChildAuthorityRequest, ChildRole, PlatformSupport, RootDelegationInput,
    RootDelegationScope,
};
use saya_types::{
    AgentId, Capabilities, Destination, FetchScope, InvocationId, JobId, ManagedCheckoutId,
    ProfileIdentity, RepositoryId, Sha256Digest, SnapshotId,
};

#[path = "checkout_write_authority/denials.rs"]
mod denials;

#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[path = "approval_support.rs"]
mod approval_support;

fn profile(value: char) -> ProfileIdentity {
    ProfileIdentity::parse(&format!("p-{}", value.to_string().repeat(64))).unwrap()
}

fn capabilities(destinations: &[(&str, &str)]) -> Capabilities {
    let mut capabilities = Capabilities::default();
    if !destinations.is_empty() {
        capabilities.fetch = Some(
            FetchScope::new(
                destinations
                    .iter()
                    .map(|(scheme, host)| Destination::new(*scheme, *host).unwrap())
                    .collect(),
            )
            .unwrap(),
        );
    }
    capabilities
}

fn write_capabilities(destinations: &[(&str, &str)]) -> Capabilities {
    let mut capabilities = capabilities(destinations);
    capabilities.workspace_write = true;
    capabilities
}

fn root(
    checkout: Option<ManagedCheckoutId>,
    capabilities: Capabilities,
    profiles: Vec<ProfileIdentity>,
) -> RootDelegationScope {
    approval_support::approved_root(RootDelegationInput {
        job_id: JobId::parse("job").unwrap(),
        agent_id: AgentId::parse("agent").unwrap(),
        invocation_id: InvocationId::parse("invocation").unwrap(),
        repository_id: RepositoryId::parse("repository").unwrap(),
        snapshot_id: SnapshotId::parse("snapshot").unwrap(),
        managed_checkout_id: checkout,
        policy_digest: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
        revocation_epoch: 7,
        capabilities,
        database_profiles: profiles,
    })
    .unwrap()
}

fn request(
    role: ChildRole,
    capabilities: Capabilities,
    profiles: Vec<ProfileIdentity>,
) -> ChildAuthorityRequest {
    ChildAuthorityRequest::new(role, capabilities, profiles).unwrap()
}

fn supported() -> PlatformSupport {
    PlatformSupport {
        workspace_edits: true,
        network_fetch: true,
        database_reads: true,
    }
}

/// There is no ceiling: a write-capable view admits any write in its own
/// checkout — `.git` included, since no path is refused — and refuses one
/// presented for another checkout, or for no checkout at all.
#[test]
fn a_write_capable_view_writes_anywhere_in_its_checkout_and_only_there() {
    let checkout = ManagedCheckoutId::parse("checkout").unwrap();
    let view = root(Some(checkout.clone()), write_capabilities(&[]), Vec::new())
        .narrow(
            request(ChildRole::Implementer, write_capabilities(&[]), Vec::new()),
            supported(),
        )
        .unwrap();

    assert!(view.workspace_write());
    assert!(view.admit_workspace_write(&checkout).is_ok());
    assert!(matches!(
        view.admit_workspace_write(&ManagedCheckoutId::parse("other-checkout").unwrap()),
        Err(AuthorityRefusal::ManagedCheckoutMismatch)
    ));

    let checkout_less = root(None, write_capabilities(&[]), Vec::new())
        .narrow(
            request(ChildRole::Implementer, write_capabilities(&[]), Vec::new()),
            supported(),
        )
        .unwrap();
    assert!(matches!(
        checkout_less.admit_workspace_write(&checkout),
        Err(AuthorityRefusal::ManagedCheckoutUnavailable)
    ));
}

/// A write role without the platform's edit support, or without either
/// side's `workspace_write` capability, holds no write at all.
#[test]
fn a_view_without_the_write_capability_refuses_every_write() {
    let checkout = ManagedCheckoutId::parse("checkout").unwrap();
    let view = root(Some(checkout.clone()), capabilities(&[]), Vec::new())
        .narrow(
            request(ChildRole::Implementer, capabilities(&[]), Vec::new()),
            supported(),
        )
        .unwrap();

    assert!(!view.workspace_write());
    assert!(matches!(
        view.admit_workspace_write(&checkout),
        Err(AuthorityRefusal::WorkspaceWriteDenied)
    ));
}

#[test]
fn analyst_and_reviewer_never_receive_write_authority() {
    let checkout = ManagedCheckoutId::parse("checkout").unwrap();
    let root = root(Some(checkout.clone()), write_capabilities(&[]), Vec::new());

    for role in [ChildRole::Analyst, ChildRole::Reviewer] {
        let view = root
            .narrow(
                request(role, write_capabilities(&[]), Vec::new()),
                supported(),
            )
            .unwrap();
        assert!(!view.workspace_write());
        assert!(matches!(
            view.admit_workspace_write(&checkout),
            Err(AuthorityRefusal::WorkspaceWriteDenied)
        ));
    }
}
