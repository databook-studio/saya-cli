//! Which profiles a coding job may use, and how a local database file path is
//! bound before consent. Both run before any connection exists: the selection
//! resolves each requested name to the identity the consent records, and
//! `bind_local_profiles` freezes the connector's cwd-relative filename
//! semantics on the job's own runtime copy.
use super::{CodingApplication, CodingError, CodingJobRequest};
use saya_types::{DatabaseProfile, ProfileIdentity};
use std::collections::BTreeMap;

impl CodingApplication {
    pub(in crate::commands::coding) fn database_selection(
        &self,
        request: &CodingJobRequest,
    ) -> Result<BTreeMap<String, ProfileIdentity>, CodingError> {
        request
            .database_profiles
            .iter()
            .map(|name| {
                let profile = self
                    .runtime
                    .connections
                    .profiles
                    .get(name)
                    .ok_or(CodingError::InvalidRequest)?;
                if matches!(profile, DatabaseProfile::Sqlite { path, .. } if path.is_empty()) {
                    return Err(CodingError::InvalidRequest);
                }
                let identity = crate::profile_identity::profile_identity(
                    name,
                    profile,
                    &self.runtime.cache_scope,
                );
                if request
                    .database_identities
                    .get(name)
                    .is_some_and(|saved| saved != &identity)
                {
                    return Err(CodingError::ApprovalRequired);
                }
                Ok((name.clone(), identity))
            })
            .collect()
    }
}

/// Freeze the existing connector's cwd-relative filename semantics for this
/// coding owner before consent/identity. Only the owned runtime copy changes.
/// Do not canonicalize files here: unselected databases must not be probed.
pub(in crate::commands::coding) fn bind_local_profiles(
    runtime: &mut crate::config::runtime::RuntimeConfig,
) -> Result<(), CodingError> {
    let cwd = std::env::current_dir().map_err(CodingError::owner)?;
    for profile in runtime.connections.profiles.values_mut() {
        let path = match profile {
            DatabaseProfile::Sqlite { path, .. } if path.as_str() != ":memory:" => path,
            _ => continue,
        };
        if !path.is_empty() && std::path::Path::new(path.as_str()).is_relative() {
            *path = cwd
                .join(&*path)
                .into_os_string()
                .into_string()
                .map_err(|_| CodingError::InvalidRequest)?;
        }
    }
    Ok(())
}
