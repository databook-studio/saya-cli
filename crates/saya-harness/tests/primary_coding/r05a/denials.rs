use saya_harness::coding::policy::{
    AuthorityError, AuthorityRefusal, ChildAuthorityRequest, ChildRole, CommandCheckoutProfile,
    PlatformSupport, RootDelegationInput,
};
use saya_types::{
    AgentId, ApprovalNetworkDestination, ApprovalNetworkScheme, Capabilities, Destination,
    FetchScope, InvocationId, JobId, ManagedCheckoutId, RepositoryId, Sha256Digest, SnapshotId,
};

use super::{
    approval_support, capabilities, path, profile, request, root, supported, write_capabilities,
};

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
