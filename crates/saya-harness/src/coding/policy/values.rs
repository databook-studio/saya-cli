use std::collections::BTreeSet;

use saya_types::{
    ApprovalNetworkDestination, ApprovalNetworkScheme, Capabilities, Destination, EndpointBindings,
    FetchScope, InterpreterScope, ProfileIdentity, RunnerScope, WorkspacePath,
};

use super::authority::{
    AuthorityError, ChildAuthorityRequest, ChildAuthorityView, ChildRole, CommandCheckoutProfile,
    MAX_DATABASE_PROFILES, MAX_WRITE_PATH_BYTES, MAX_WRITE_PATH_TOTAL_BYTES, MAX_WRITE_PATHS,
    PlatformSupport, RootDelegationScope,
};

pub(super) fn validate_paths(paths: &[WorkspacePath]) -> Result<(), AuthorityError> {
    if paths.len() > MAX_WRITE_PATHS {
        return Err(AuthorityError::TooManyWritePaths);
    }
    let mut total = 0usize;
    let mut seen = BTreeSet::new();
    for path in paths {
        let value = path.as_str();
        if value.len() > MAX_WRITE_PATH_BYTES {
            return Err(AuthorityError::WritePathTooLong);
        }
        total += value.len();
        if total > MAX_WRITE_PATH_TOTAL_BYTES {
            return Err(AuthorityError::WritePathsTooLarge);
        }
        if !seen.insert(value) {
            return Err(AuthorityError::DuplicateWritePath);
        }
    }
    Ok(())
}

pub(super) fn validate_profiles(
    profiles: Vec<ProfileIdentity>,
) -> Result<Vec<ProfileIdentity>, AuthorityError> {
    if profiles.len() > MAX_DATABASE_PROFILES {
        return Err(AuthorityError::TooManyDatabaseProfiles);
    }
    let mut seen = BTreeSet::new();
    if profiles
        .iter()
        .any(|profile| !seen.insert(profile.as_str()))
    {
        return Err(AuthorityError::DuplicateDatabaseProfile);
    }
    Ok(profiles)
}

pub(super) fn validate_capabilities(
    capabilities: &Capabilities,
) -> Result<Capabilities, AuthorityError> {
    let fetch = capabilities
        .fetch
        .as_ref()
        .map(validate_fetch)
        .transpose()?;
    let runner = capabilities
        .runner
        .as_ref()
        .map(|scope| RunnerScope::new(scope.programs.clone()))
        .transpose()
        .map_err(|_| AuthorityError::InvalidCapabilityShape)?;
    let interpreter = capabilities
        .interpreter
        .as_ref()
        .map(|scope| InterpreterScope::new(scope.programs.clone()))
        .transpose()
        .map_err(|_| AuthorityError::InvalidCapabilityShape)?;
    let endpoints = EndpointBindings::new(
        capabilities
            .endpoints
            .as_map()
            .iter()
            .map(|(role, endpoint)| (role.clone(), endpoint.clone())),
    )
    .map_err(|_| AuthorityError::InvalidCapabilityShape)?;
    let mut validated = Capabilities::default();
    validated.workspace_write = capabilities.workspace_write;
    validated.fetch = fetch;
    validated.runner = runner;
    validated.interpreter = interpreter;
    validated.scratch = capabilities.scratch;
    validated.endpoints = endpoints;
    Ok(validated)
}

fn validate_fetch(scope: &FetchScope) -> Result<FetchScope, AuthorityError> {
    let destinations = scope
        .destinations
        .iter()
        .map(validate_destination)
        .collect::<Result<Vec<_>, _>>()?;
    FetchScope::new(destinations).map_err(|_| AuthorityError::InvalidCapabilityShape)
}

fn validate_destination(destination: &Destination) -> Result<Destination, AuthorityError> {
    let scheme = match destination.scheme.as_str() {
        "http" => ApprovalNetworkScheme::Http,
        "https" => ApprovalNetworkScheme::Https,
        _ => return Err(AuthorityError::InvalidCapabilityShape),
    };
    let strict = ApprovalNetworkDestination::new(scheme, destination.host.clone())
        .map_err(|_| AuthorityError::InvalidCapabilityShape)?;
    let scheme = match strict.scheme() {
        ApprovalNetworkScheme::Http => "http",
        ApprovalNetworkScheme::Https => "https",
        _ => return Err(AuthorityError::InvalidCapabilityShape),
    };
    Destination::new(scheme, strict.host()).map_err(|_| AuthorityError::InvalidCapabilityShape)
}

pub(super) fn narrow(
    root: &RootDelegationScope,
    request: ChildAuthorityRequest,
    platform: PlatformSupport,
) -> Result<ChildAuthorityView, AuthorityError> {
    let commands_allowed = match request.role {
        ChildRole::Analyst | ChildRole::Reviewer => {
            platform.command_profile == Some(CommandCheckoutProfile::ReadOnlyCheckout)
        }
        ChildRole::Implementer => {
            platform.command_profile == Some(CommandCheckoutProfile::IsolatedPrivateCheckout)
        }
    };
    let writes_allowed = request.role == ChildRole::Implementer
        && platform.workspace_edits
        && root.capabilities.workspace_write
        && request.capabilities.workspace_write
        && root.managed_checkout_id.is_some();
    let write_paths = if writes_allowed {
        intersect(&root.write_paths, &request.write_paths)
    } else {
        Vec::new()
    };
    Ok(ChildAuthorityView {
        job_id: root.job_id.clone(),
        agent_id: root.agent_id.clone(),
        invocation_id: root.invocation_id.clone(),
        repository_id: root.repository_id.clone(),
        snapshot_id: root.snapshot_id.clone(),
        managed_checkout_id: root.managed_checkout_id.clone(),
        policy_digest: root.policy_digest.clone(),
        revocation_epoch: root.revocation_epoch,
        write_paths,
        capabilities: intersect_capabilities(
            &root.capabilities,
            &request.capabilities,
            commands_allowed,
            platform.network_fetch,
        )?,
        database_profiles: if platform.database_reads {
            intersect(&root.database_profiles, &request.database_profiles)
        } else {
            Vec::new()
        },
    })
}

fn intersect_capabilities(
    root: &Capabilities,
    request: &Capabilities,
    commands_allowed: bool,
    network_allowed: bool,
) -> Result<Capabilities, AuthorityError> {
    let fetch = network_allowed
        .then(|| intersect_fetch(root, request))
        .flatten();
    let runner = commands_allowed
        .then(|| intersect_runner(root, request))
        .flatten();
    let interpreter = commands_allowed
        .then(|| intersect_interpreter(root, request))
        .flatten();
    let endpoints = EndpointBindings::new(
        root.endpoints
            .as_map()
            .iter()
            .filter(|&(role, endpoint)| request.endpoints.get(role) == Some(endpoint.as_str()))
            .map(|(role, endpoint)| (role.clone(), endpoint.clone())),
    )
    .map_err(|_| AuthorityError::InvalidCapabilityShape)?;
    let mut intersected = Capabilities::default();
    intersected.workspace_write = false;
    intersected.fetch = fetch;
    intersected.runner = runner;
    intersected.interpreter = interpreter;
    intersected.scratch = false;
    intersected.endpoints = endpoints;
    Ok(intersected)
}

fn intersect_fetch(root: &Capabilities, request: &Capabilities) -> Option<FetchScope> {
    let (Some(root), Some(request)) = (&root.fetch, &request.fetch) else {
        return None;
    };
    FetchScope::new(intersect(&root.destinations, &request.destinations)).ok()
}

fn intersect_runner(root: &Capabilities, request: &Capabilities) -> Option<RunnerScope> {
    let (Some(root), Some(request)) = (&root.runner, &request.runner) else {
        return None;
    };
    RunnerScope::new(intersect(&root.programs, &request.programs)).ok()
}

fn intersect_interpreter(root: &Capabilities, request: &Capabilities) -> Option<InterpreterScope> {
    let (Some(root), Some(request)) = (&root.interpreter, &request.interpreter) else {
        return None;
    };
    InterpreterScope::new(intersect(&root.programs, &request.programs)).ok()
}

fn intersect<T: Clone + PartialEq>(root: &[T], request: &[T]) -> Vec<T> {
    root.iter()
        .filter(|value| request.contains(value))
        .cloned()
        .collect()
}
