use saya_harness::coding::policy::{
    AuthorityError, ChildAuthorityRequest, ChildRole, CommandCheckoutProfile, PlatformSupport,
    RootDelegationInput, RootDelegationScope,
};
use saya_types::{
    AgentId, ApprovalNetworkDestination, ApprovalNetworkScheme, Capabilities, Destination,
    EndpointBindings, FetchScope, InterpreterScope, InvocationId, JobId, ManagedCheckoutId,
    ProfileIdentity, RepositoryId, RunnerScope, Sha256Digest, SnapshotId, WorkspacePath,
};

fn path(value: impl Into<String>) -> WorkspacePath {
    WorkspacePath::try_from(value.into()).unwrap()
}

fn path_set(count: usize, bytes: usize) -> Vec<WorkspacePath> {
    (0..count)
        .map(|index| {
            let prefix = format!("p{index:03}/");
            path(format!("{prefix}{}", "x".repeat(bytes - prefix.len())))
        })
        .collect()
}

fn profile(index: usize) -> ProfileIdentity {
    ProfileIdentity::parse(&format!("p-{index:064x}")).unwrap()
}

fn root(
    capabilities: Capabilities,
    write_paths: Vec<WorkspacePath>,
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
        write_paths,
        capabilities,
        database_profiles,
    })
}

fn command_capabilities() -> Capabilities {
    let mut capabilities = Capabilities::default();
    capabilities.runner = Some(RunnerScope::new(vec!["git".into()]).unwrap());
    capabilities.interpreter = Some(InterpreterScope::new(vec!["python3".into()]).unwrap());
    capabilities
}

fn command_support(profile: CommandCheckoutProfile) -> PlatformSupport {
    PlatformSupport {
        command_profile: Some(profile),
        ..PlatformSupport::default()
    }
}

#[test]
fn command_profiles_gate_roles_and_both_program_families() {
    let root = root(command_capabilities(), Vec::new(), Vec::new()).unwrap();
    for role in [ChildRole::Analyst, ChildRole::Reviewer] {
        let view = root
            .narrow(
                ChildAuthorityRequest::new(role, Vec::new(), command_capabilities(), Vec::new())
                    .unwrap(),
                command_support(CommandCheckoutProfile::ReadOnlyCheckout),
            )
            .unwrap();
        assert!(view.admit_runner_program("git").is_ok());
        assert!(view.admit_interpreter_program("python3").is_ok());

        let unsupported_profile = root
            .narrow(
                ChildAuthorityRequest::new(role, Vec::new(), command_capabilities(), Vec::new())
                    .unwrap(),
                command_support(CommandCheckoutProfile::IsolatedPrivateCheckout),
            )
            .unwrap();
        assert!(unsupported_profile.capabilities().runner.is_none());
        assert!(unsupported_profile.capabilities().interpreter.is_none());
    }

    let implementer_read_only = root
        .narrow(
            ChildAuthorityRequest::new(
                ChildRole::Implementer,
                Vec::new(),
                command_capabilities(),
                Vec::new(),
            )
            .unwrap(),
            command_support(CommandCheckoutProfile::ReadOnlyCheckout),
        )
        .unwrap();
    assert!(matches!(
        implementer_read_only.admit_runner_program("git"),
        Err(saya_harness::coding::policy::AuthorityRefusal::ProgramDenied)
    ));
    assert!(matches!(
        implementer_read_only.admit_interpreter_program("python3"),
        Err(saya_harness::coding::policy::AuthorityRefusal::ProgramDenied)
    ));

    let implementer_private = root
        .narrow(
            ChildAuthorityRequest::new(
                ChildRole::Implementer,
                Vec::new(),
                command_capabilities(),
                Vec::new(),
            )
            .unwrap(),
            command_support(CommandCheckoutProfile::IsolatedPrivateCheckout),
        )
        .unwrap();
    assert!(implementer_private.admit_runner_program("git").is_ok());
    assert!(
        implementer_private
            .admit_interpreter_program("python3")
            .is_ok()
    );
}

#[test]
fn endpoint_and_scratch_metadata_are_narrowed_and_legacy_hosts_canonicalized() {
    let mut root_capabilities = Capabilities::default();
    root_capabilities.scratch = true;
    root_capabilities.endpoints =
        EndpointBindings::new([("orchestrator", "primary"), ("reviewer", "trusted-review")])
            .unwrap();
    root_capabilities.fetch =
        Some(FetchScope::new(vec![Destination::new("https", "DB.Example.Test").unwrap()]).unwrap());

    let mut request_capabilities = Capabilities::default();
    request_capabilities.scratch = true;
    request_capabilities.endpoints = EndpointBindings::new([
        ("orchestrator", "primary"),
        ("reviewer", "requested-review"),
        ("extra", "untrusted-extra"),
    ])
    .unwrap();
    request_capabilities.fetch =
        Some(FetchScope::new(vec![Destination::new("https", "db.example.test").unwrap()]).unwrap());

    let view = root(root_capabilities, Vec::new(), Vec::new())
        .unwrap()
        .narrow(
            ChildAuthorityRequest::new(
                ChildRole::Analyst,
                Vec::new(),
                request_capabilities,
                Vec::new(),
            )
            .unwrap(),
            PlatformSupport {
                network_fetch: true,
                ..PlatformSupport::default()
            },
        )
        .unwrap();

    let capabilities = view.capabilities();
    assert!(!capabilities.scratch);
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

#[test]
fn public_constructors_enforce_path_and_profile_set_bounds() {
    let empty_caps = Capabilities::default();

    // Exact path count and aggregate-byte limits are accepted.
    assert!(root(empty_caps.clone(), path_set(128, 16), Vec::new()).is_ok());
    assert!(root(empty_caps.clone(), path_set(64, 512), Vec::new()).is_ok());
    assert_eq!(
        root(empty_caps.clone(), path_set(129, 16), Vec::new()).unwrap_err(),
        AuthorityError::TooManyWritePaths
    );
    assert_eq!(
        root(empty_caps.clone(), path_set(65, 512), Vec::new()).unwrap_err(),
        AuthorityError::WritePathsTooLarge
    );

    // Requests are subject to the same duplicate-path and profile constraints.
    assert_eq!(
        ChildAuthorityRequest::new(
            ChildRole::Implementer,
            vec![path("src/a.rs"), path("src/a.rs")],
            empty_caps.clone(),
            Vec::new(),
        )
        .unwrap_err(),
        AuthorityError::DuplicateWritePath
    );
    let profiles = (0..32).map(profile).collect::<Vec<_>>();
    assert!(
        ChildAuthorityRequest::new(
            ChildRole::Reviewer,
            Vec::new(),
            empty_caps.clone(),
            profiles.clone(),
        )
        .is_ok()
    );
    let mut too_many_profiles = profiles;
    too_many_profiles.push(profile(32));
    assert_eq!(
        ChildAuthorityRequest::new(
            ChildRole::Reviewer,
            Vec::new(),
            empty_caps.clone(),
            too_many_profiles,
        )
        .unwrap_err(),
        AuthorityError::TooManyDatabaseProfiles
    );
    assert_eq!(
        root(empty_caps.clone(), Vec::new(), vec![profile(1), profile(1)],).unwrap_err(),
        AuthorityError::DuplicateDatabaseProfile
    );
}

#[path = "approval_support.rs"]
mod approval_support;
