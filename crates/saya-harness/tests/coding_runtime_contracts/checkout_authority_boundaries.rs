use saya_harness::coding::policy::{
    AuthorityError, ChildAuthorityRequest, ChildRole, PlatformSupport, RootDelegationInput,
    RootDelegationScope,
};
use saya_types::{
    AgentId, ApprovalNetworkDestination, ApprovalNetworkScheme, Capabilities, Destination,
    EndpointBindings, FetchScope, InvocationId, JobId, ManagedCheckoutId, ProfileIdentity,
    RepositoryId, Sha256Digest, SnapshotId,
};

fn profile(index: usize) -> ProfileIdentity {
    ProfileIdentity::parse(&format!("p-{index:064x}")).unwrap()
}

fn root(
    capabilities: Capabilities,
    database_profiles: Vec<ProfileIdentity>,
) -> Result<RootDelegationScope, AuthorityError> {
    approval_support::approved_root(RootDelegationInput {
        job_id: JobId::parse("job").unwrap(),
        agent_id: AgentId::parse("agent").unwrap(),
        invocation_id: InvocationId::parse("invocation").unwrap(),
        repository_id: RepositoryId::parse("repository").unwrap(),
        snapshot_id: SnapshotId::parse("snapshot").unwrap(),
        managed_checkout_id: Some(ManagedCheckoutId::parse("checkout").unwrap()),
        policy_digest: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
        revocation_epoch: 7,
        capabilities,
        database_profiles,
    })
}

#[test]
fn endpoint_metadata_is_narrowed_and_legacy_hosts_canonicalized() {
    let mut root_capabilities = Capabilities::default();
    root_capabilities.endpoints =
        EndpointBindings::new([("orchestrator", "primary"), ("reviewer", "trusted-review")])
            .unwrap();
    root_capabilities.fetch =
        Some(FetchScope::new(vec![Destination::new("https", "DB.Example.Test").unwrap()]).unwrap());

    let mut request_capabilities = Capabilities::default();
    request_capabilities.endpoints = EndpointBindings::new([
        ("orchestrator", "primary"),
        ("reviewer", "requested-review"),
        ("extra", "untrusted-extra"),
    ])
    .unwrap();
    request_capabilities.fetch =
        Some(FetchScope::new(vec![Destination::new("https", "db.example.test").unwrap()]).unwrap());

    let view = root(root_capabilities, Vec::new())
        .unwrap()
        .narrow(
            ChildAuthorityRequest::new(ChildRole::Analyst, request_capabilities, Vec::new())
                .unwrap(),
            PlatformSupport {
                network_fetch: true,
                ..PlatformSupport::default()
            },
        )
        .unwrap();

    let capabilities = view.capabilities();
    assert_eq!(capabilities.endpoints.as_map().len(), 1);
    assert_eq!(capabilities.endpoints.get("orchestrator"), Some("primary"));
    assert!(!capabilities.endpoints.contains("reviewer"));
    assert!(!capabilities.endpoints.contains("extra"));
    let fetch = capabilities.fetch.as_ref().unwrap();
    assert_eq!(fetch.destinations.len(), 1);
    assert_eq!(fetch.destinations[0].host, "db.example.test");
    assert!(
        view.admit_network_destination(
            &ApprovalNetworkDestination::new(ApprovalNetworkScheme::Https, "DB.EXAMPLE.TEST")
                .unwrap()
        )
        .is_ok()
    );
}

/// There are no write-path bounds: the surviving shape bounds are the
/// database profile set's, on both constructors.
#[test]
fn public_constructors_enforce_profile_set_bounds() {
    let empty_caps = Capabilities::default();

    let profiles = (0..32).map(profile).collect::<Vec<_>>();
    assert!(
        ChildAuthorityRequest::new(ChildRole::Reviewer, empty_caps.clone(), profiles.clone())
            .is_ok()
    );
    let mut too_many_profiles = profiles;
    too_many_profiles.push(profile(32));
    assert_eq!(
        ChildAuthorityRequest::new(ChildRole::Reviewer, empty_caps.clone(), too_many_profiles)
            .unwrap_err(),
        AuthorityError::TooManyDatabaseProfiles
    );
    assert_eq!(
        root(empty_caps.clone(), vec![profile(1), profile(1)]).unwrap_err(),
        AuthorityError::DuplicateDatabaseProfile
    );
}

#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[path = "approval_support.rs"]
mod approval_support;
