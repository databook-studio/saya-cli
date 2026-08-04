use super::runtime::AgentRuntimeError;
use crate::config::runtime::RuntimeConfig;

pub(crate) fn selected(
    runtime: &RuntimeConfig,
    override_name: Option<&String>,
) -> Result<(Option<String>, Option<saya_types::DatabaseProfile>), AgentRuntimeError> {
    match override_name {
        Some(name) => runtime
            .named_profile(name)
            .map(|profile| (Some(name.clone()), Some(profile.clone())))
            .map_err(|error| AgentRuntimeError::Configuration(error.to_string())),
        None => Ok((
            runtime.resolved.profile_name.clone(),
            runtime.resolved.profile.clone(),
        )),
    }
}
