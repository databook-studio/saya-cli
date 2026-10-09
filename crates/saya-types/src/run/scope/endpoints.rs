//! Endpoint bindings: the roles a run may call and the endpoints they are
//! bound to, bounded so a plan cannot smuggle an unbounded approval view.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::naming::is_name_shaped;
use crate::run::RunContractError;

pub const MAX_ENDPOINT_BINDINGS: usize = 8;

/// Roles bound to named endpoints, e.g. `orchestrator → primary`. Both sides
/// of a binding are run-scoped names; entries are bounded so a plan cannot
/// smuggle an unbounded approval view.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(try_from = "BTreeMap<String, String>")]
pub struct EndpointBindings(BTreeMap<String, String>);

impl EndpointBindings {
    pub fn new<K: Into<String>, V: Into<String>>(
        bindings: impl IntoIterator<Item = (K, V)>,
    ) -> Result<Self, RunContractError> {
        let mut map = BTreeMap::new();
        for (role, endpoint) in bindings {
            let (role, endpoint) = (role.into(), endpoint.into());
            if !is_name_shaped(&role) || !is_name_shaped(&endpoint) {
                return Err(RunContractError::InvalidEndpointName);
            }
            map.insert(role, endpoint);
        }
        if map.len() > MAX_ENDPOINT_BINDINGS {
            return Err(RunContractError::TooManyEndpointBindings);
        }
        Ok(Self(map))
    }

    pub fn get(&self, role: &str) -> Option<&str> {
        self.0.get(role).map(String::as_str)
    }

    pub fn contains(&self, role: &str) -> bool {
        self.0.contains_key(role)
    }

    pub fn as_map(&self) -> &BTreeMap<String, String> {
        &self.0
    }

    pub fn is_subset_of(&self, approved: &Self) -> bool {
        self.0
            .iter()
            .all(|(role, endpoint)| approved.0.get(role) == Some(endpoint))
    }
}

impl TryFrom<BTreeMap<String, String>> for EndpointBindings {
    type Error = RunContractError;

    fn try_from(map: BTreeMap<String, String>) -> Result<Self, Self::Error> {
        Self::new(map)
    }
}
