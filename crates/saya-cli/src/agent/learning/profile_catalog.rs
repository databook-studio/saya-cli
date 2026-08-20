//! Per-extraction cache of each profile's identity and live schema.
//!
//! Resolving an under-qualified object name needs the profile's real catalog,
//! and a turn can carry up to `MAX_PROPOSALS_PER_EXTRACTION` proposals. Without
//! a cache each one would drive a fresh catalog introspection against a live
//! database on the post-turn path — eight full introspections for one sentence,
//! and on a warehouse that is a real cost. The schema is read once per profile
//! per extraction and reused; it is scoped to the run, so a schema change is
//! picked up on the next turn.

use crate::connection::ConnectionRegistry;
use saya_types::{ProfileIdentity, SchemaTree};
use std::collections::HashMap;

/// What resolution needs to know about one profile.
pub(crate) struct ProfileFacts {
    pub identity: ProfileIdentity,
    pub schema: SchemaTree,
}

/// Reason a profile could not be described. Mirrors the resolver's error cases
/// so the caller can map them without this module depending on the resolver.
#[derive(Debug, Clone)]
pub(crate) enum ProfileLookupError {
    Connection(String),
    MissingIdentity,
    InvalidIdentity(String),
}

/// Caches `(identity, schema)` per profile name for the life of one extraction.
#[derive(Default)]
pub(crate) struct ProfileCatalog {
    seen: HashMap<String, Result<ProfileFacts, ProfileLookupError>>,
}

impl ProfileCatalog {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Returns the profile's identity and schema, reading them at most once.
    ///
    /// A failure is cached too: a profile that could not be reached will not be
    /// retried for every remaining proposal in the same turn.
    pub(crate) async fn facts(
        &mut self,
        profile: &str,
        registry: &ConnectionRegistry,
    ) -> Result<&ProfileFacts, ProfileLookupError> {
        if !self.seen.contains_key(profile) {
            let looked_up = Self::look_up(profile, registry).await;
            self.seen.insert(profile.to_string(), looked_up);
        }
        match self.seen.get(profile) {
            Some(Ok(facts)) => Ok(facts),
            Some(Err(error)) => Err(error.clone()),
            // The insert above guarantees an entry; treat its absence as a
            // lookup failure rather than panicking on the post-turn path.
            None => Err(ProfileLookupError::MissingIdentity),
        }
    }

    async fn look_up(
        profile: &str,
        registry: &ConnectionRegistry,
    ) -> Result<ProfileFacts, ProfileLookupError> {
        let entry = registry
            .resolve(Some(profile))
            .map_err(|e| ProfileLookupError::Connection(e.to_string()))?;

        let identity = entry
            .profile_id
            .as_deref()
            .ok_or(ProfileLookupError::MissingIdentity)
            .and_then(|raw| {
                ProfileIdentity::parse(raw)
                    .map_err(|e| ProfileLookupError::InvalidIdentity(e.to_string()))
            })?;

        let schema = entry
            .connector
            .schema()
            .await
            .map_err(|e| ProfileLookupError::Connection(e.to_string()))?;

        Ok(ProfileFacts { identity, schema })
    }
}
