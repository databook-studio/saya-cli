use saya_harness::coding::policy::{
    AuthorityError, AuthorityRefusal, ChildAuthorityRequest, ChildRole, CommandCheckoutProfile,
    PlatformSupport, RootDelegationInput, RootDelegationScope,
};
use saya_types::{
    AgentId, ApprovalNetworkDestination, ApprovalNetworkScheme, Capabilities, Destination,
    FetchScope, InvocationId, JobId, ManagedCheckoutId, ProfileIdentity, RepositoryId, RunnerScope,
    Sha256Digest, SnapshotId, WorkspacePath,
};

fn path(value: &str) -> WorkspacePath {
    WorkspacePath::try_from(value.to_owned()).unwrap()
}

fn profile(value: char) -> ProfileIdentity {
    ProfileIdentity::parse(&format!("p-{}", value.to_string().repeat(64))).unwrap()
}

fn capabilities(programs: &[&str], destinations: &[(&str, &str)]) -> Capabilities {
    let mut capabilities = Capabilities::default();
    if !programs.is_empty() {
        capabilities.runner = Some(
            RunnerScope::new(
                programs
                    .iter()
                    .map(|program| (*program).to_owned())
                    .collect(),
            )
            .unwrap(),
        );
    }
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

fn write_capabilities(programs: &[&str], destinations: &[(&str, &str)]) -> Capabilities {
    let mut capabilities = capabilities(programs, destinations);
    capabilities.workspace_write = true;
    capabilities
}

fn root(
    checkout: Option<ManagedCheckoutId>,
    paths: Vec<WorkspacePath>,
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
        write_paths: paths,
        capabilities,
        database_profiles: profiles,
    })
    .unwrap()
}

fn request(
    role: ChildRole,
    paths: Vec<WorkspacePath>,
    capabilities: Capabilities,
    profiles: Vec<ProfileIdentity>,
) -> ChildAuthorityRequest {
    ChildAuthorityRequest::new(role, paths, capabilities, profiles).unwrap()
}

fn supported() -> PlatformSupport {
    PlatformSupport {
        workspace_edits: true,
        command_profile: Some(CommandCheckoutProfile::IsolatedPrivateCheckout),
        network_fetch: true,
        database_reads: true,
    }
}

#[test]
fn root_ceiling_denies_requested_effects_outside_its_domains() {
    let checkout = ManagedCheckoutId::parse("checkout").unwrap();
    let allowed_profile = profile('a');
    let view = root(
        Some(checkout.clone()),
        vec![path("src/allowed.rs")],
        write_capabilities(&["git"], &[("https", "allowed.example")]),
        vec![allowed_profile.clone()],
    )
    .narrow(
        request(
            ChildRole::Implementer,
            vec![path("src/allowed.rs"), path("src/requested.rs")],
            write_capabilities(
                &["git", "curl"],
                &[("https", "allowed.example"), ("https", "requested.example")],
            ),
            vec![allowed_profile.clone(), profile('b')],
        ),
        supported(),
    )
    .unwrap();

    assert!(
        view.admit_workspace_write(&checkout, &path("src/allowed.rs"))
            .is_ok()
    );
    assert!(matches!(
        view.admit_workspace_write(&checkout, &path("src/requested.rs")),
        Err(AuthorityRefusal::WorkspaceWriteDenied)
    ));
    assert!(view.admit_runner_program("git").is_ok());
    assert!(matches!(
        view.admit_runner_program("curl"),
        Err(AuthorityRefusal::ProgramDenied)
    ));
    assert!(
        view.admit_network_destination(
            &ApprovalNetworkDestination::new(ApprovalNetworkScheme::Https, "allowed.example")
                .unwrap()
        )
        .is_ok()
    );
    assert!(matches!(
        view.admit_network_destination(
            &ApprovalNetworkDestination::new(ApprovalNetworkScheme::Https, "requested.example")
                .unwrap()
        ),
        Err(AuthorityRefusal::NetworkDestinationDenied)
    ));
    assert!(view.admit_database_profile(&allowed_profile).is_ok());
    assert!(matches!(
        view.admit_database_profile(&profile('b')),
        Err(AuthorityRefusal::DatabaseProfileDenied)
    ));
}

#[test]
fn writes_are_exact_and_bound_to_the_managed_checkout() {
    let checkout = ManagedCheckoutId::parse("checkout").unwrap();
    let view = root(
        Some(checkout.clone()),
        vec![path("src/owned.rs")],
        write_capabilities(&[], &[]),
        Vec::new(),
    )
    .narrow(
        request(
            ChildRole::Implementer,
            vec![path("src/owned.rs"), path("src/not-root-owned.rs")],
            write_capabilities(&[], &[]),
            Vec::new(),
        ),
        supported(),
    )
    .unwrap();

    assert_eq!(view.write_paths(), &[path("src/owned.rs")]);
    assert!(matches!(
        view.admit_workspace_write(
            &ManagedCheckoutId::parse("other-checkout").unwrap(),
            &path("src/owned.rs")
        ),
        Err(AuthorityRefusal::ManagedCheckoutMismatch)
    ));
    assert!(matches!(
        view.admit_workspace_write(&checkout, &path("src/not-root-owned.rs")),
        Err(AuthorityRefusal::WorkspaceWriteDenied)
    ));
}

#[test]
fn analyst_and_reviewer_never_receive_source_write_authority() {
    let checkout = ManagedCheckoutId::parse("checkout").unwrap();
    let root = root(
        Some(checkout.clone()),
        vec![path("src/owned.rs")],
        write_capabilities(&[], &[]),
        Vec::new(),
    );

    for role in [ChildRole::Analyst, ChildRole::Reviewer] {
        let view = root
            .narrow(
                request(
                    role,
                    vec![path("src/owned.rs")],
                    write_capabilities(&[], &[]),
                    Vec::new(),
                ),
                supported(),
            )
            .unwrap();
        assert!(view.write_paths().is_empty());
        assert!(matches!(
            view.admit_workspace_write(&checkout, &path("src/owned.rs")),
            Err(AuthorityRefusal::WorkspaceWriteDenied)
        ));
    }
}

#[test]
fn intersections_reject_malformed_destinations_and_unmatched_domains() {
    let mut malformed = Capabilities::default();
    let mut malformed_fetch =
        FetchScope::new(vec![Destination::new("https", "example.test").unwrap()]).unwrap();
    malformed_fetch.destinations[0].host = "example.test:8443".to_owned();
    malformed.fetch = Some(malformed_fetch);
    assert!(matches!(
        approval_support::approved_root(RootDelegationInput {
            job_id: JobId::parse("job").unwrap(),
            agent_id: AgentId::parse("agent").unwrap(),
            invocation_id: InvocationId::parse("invocation").unwrap(),
            repository_id: RepositoryId::parse("repository").unwrap(),
            snapshot_id: SnapshotId::parse("snapshot").unwrap(),
            managed_checkout_id: None,
            policy_digest: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
            revocation_epoch: 7,
            write_paths: Vec::new(),
            capabilities: malformed.clone(),
            database_profiles: Vec::new(),
        }),
        Err(AuthorityError::InvalidCapabilityShape)
    ));
    assert!(matches!(
        ChildAuthorityRequest::new(ChildRole::Reviewer, Vec::new(), malformed, Vec::new(),),
        Err(AuthorityError::InvalidCapabilityShape)
    ));

    let view = root(
        Some(ManagedCheckoutId::parse("checkout").unwrap()),
        Vec::new(),
        capabilities(&["git"], &[("https", "allowed.example")]),
        vec![profile('a')],
    )
    .narrow(
        request(
            ChildRole::Reviewer,
            Vec::new(),
            capabilities(&["curl"], &[("https", "requested.example")]),
            vec![profile('b')],
        ),
        PlatformSupport {
            command_profile: Some(CommandCheckoutProfile::ReadOnlyCheckout),
            ..supported()
        },
    )
    .unwrap();
    assert!(matches!(
        view.admit_runner_program("git"),
        Err(AuthorityRefusal::ProgramDenied)
    ));
    assert!(matches!(
        view.admit_network_destination(
            &ApprovalNetworkDestination::new(ApprovalNetworkScheme::Https, "allowed.example")
                .unwrap()
        ),
        Err(AuthorityRefusal::NetworkDestinationDenied)
    ));
    assert!(matches!(
        view.admit_database_profile(&profile('a')),
        Err(AuthorityRefusal::DatabaseProfileDenied)
    ));
}

#[test]
fn unsupported_platform_strips_otherwise_authorized_effects() {
    let checkout = ManagedCheckoutId::parse("checkout").unwrap();
    let allowed_profile = profile('a');
    let view = root(
        Some(checkout.clone()),
        vec![path("src/owned.rs")],
        write_capabilities(&["git"], &[("https", "allowed.example")]),
        vec![allowed_profile.clone()],
    )
    .narrow(
        request(
            ChildRole::Implementer,
            vec![path("src/owned.rs")],
            write_capabilities(&["git"], &[("https", "allowed.example")]),
            vec![allowed_profile.clone()],
        ),
        PlatformSupport::default(),
    )
    .unwrap();

    assert!(view.write_paths().is_empty());
    assert!(matches!(
        view.admit_workspace_write(&checkout, &path("src/owned.rs")),
        Err(AuthorityRefusal::WorkspaceWriteDenied)
    ));
    assert!(matches!(
        view.admit_runner_program("git"),
        Err(AuthorityRefusal::ProgramDenied)
    ));
    assert!(matches!(
        view.admit_network_destination(
            &ApprovalNetworkDestination::new(ApprovalNetworkScheme::Https, "allowed.example")
                .unwrap()
        ),
        Err(AuthorityRefusal::NetworkDestinationDenied)
    ));
    assert!(matches!(
        view.admit_database_profile(&allowed_profile),
        Err(AuthorityRefusal::DatabaseProfileDenied)
    ));
}

#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[path = "approval_support.rs"]
mod approval_support;
