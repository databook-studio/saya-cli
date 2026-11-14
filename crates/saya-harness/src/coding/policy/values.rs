use std::collections::BTreeSet;

use saya_types::{
    ApprovalNetworkDestination, ApprovalNetworkScheme, Capabilities, Destination, EndpointBindings,
    FetchScope, ProfileIdentity,
};

use super::authority::{
    AuthorityError, ChildAuthorityRequest, ChildAuthorityView, ChildRole, MAX_DATABASE_PROFILES,
    PlatformSupport, RootDelegationScope,
};

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
    // The write grant is a role fact: a write role on a platform that
    // permits edits, with both the root and the request granting
    // `workspace_write` and a shared working checkout to write. The declared
    // path sets narrow nothing, so they are not consulted here.
    let writes_allowed = request.role == ChildRole::Implementer
        && platform.workspace_edits
        && root.capabilities.workspace_write
        && request.capabilities.workspace_write
        && root.managed_checkout_id.is_some();
    Ok(ChildAuthorityView {
        job_id: root.job_id.clone(),
        agent_id: root.agent_id.clone(),
        invocation_id: root.invocation_id.clone(),
        repository_id: root.repository_id.clone(),
        snapshot_id: root.snapshot_id.clone(),
        managed_checkout_id: root.managed_checkout_id.clone(),
        policy_digest: root.policy_digest.clone(),
        revocation_epoch: root.revocation_epoch,
        capabilities: intersect_capabilities(
            &root.capabilities,
            &request.capabilities,
            platform.network_fetch,
            writes_allowed,
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
    network_allowed: bool,
    writes_allowed: bool,
) -> Result<Capabilities, AuthorityError> {
    let fetch = network_allowed
        .then(|| intersect_fetch(root, request))
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
    intersected.workspace_write = writes_allowed;
    intersected.fetch = fetch;
    intersected.endpoints = endpoints;
    Ok(intersected)
}

fn intersect_fetch(root: &Capabilities, request: &Capabilities) -> Option<FetchScope> {
    let (Some(root), Some(request)) = (&root.fetch, &request.fetch) else {
        return None;
    };
    FetchScope::new(intersect(&root.destinations, &request.destinations)).ok()
}

fn intersect<T: Clone + PartialEq>(root: &[T], request: &[T]) -> Vec<T> {
    root.iter()
        .filter(|value| request.contains(value))
        .cloned()
        .collect()
}
