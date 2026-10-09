//! The fetch capability: one declared destination and the scope that holds
//! the destinations a run may reach.

use serde::{Deserialize, Serialize};

use crate::run::RunContractError;

pub const MAX_FETCH_DESTINATIONS: usize = 32;

const MAX_HOST_CHARS: usize = 253;

/// One declared fetch destination: a scheme plus a bare host. The fetch
/// policy (a later milestone) decides which schemes and hosts are actually
/// reachable; this contract only demands a shape that cannot smuggle a path,
/// credentials, or control characters into the approval view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Destination {
    /// URL scheme, e.g. `https` — never a full URL.
    pub scheme: String,
    /// Hostname only: no scheme, path, userinfo, or port.
    pub host: String,
}

impl Destination {
    pub fn new(
        scheme: impl Into<String>,
        host: impl Into<String>,
    ) -> Result<Self, RunContractError> {
        let scheme = scheme.into();
        let host = host.into();
        let scheme_shaped = !scheme.is_empty()
            && scheme.len() <= 32
            && scheme.bytes().all(|b| b.is_ascii_alphanumeric());
        let host_shaped = !host.is_empty()
            && host.chars().count() <= MAX_HOST_CHARS
            && !host
                .chars()
                .any(|c| c.is_control() || c.is_whitespace() || c == '/' || c == '\\');
        if !scheme_shaped || !host_shaped {
            return Err(RunContractError::InvalidDestination);
        }
        Ok(Self { scheme, host })
    }
}

/// The fetch capability with its declared destinations. A fetch scope with no
/// destinations is a declared capability that can reach nothing, so it is
/// rejected — egress is approved per destination or not at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FetchScope {
    pub destinations: Vec<Destination>,
}

impl FetchScope {
    pub fn new(destinations: Vec<Destination>) -> Result<Self, RunContractError> {
        if destinations.is_empty() {
            return Err(RunContractError::EmptyDestinations);
        }
        if destinations.len() > MAX_FETCH_DESTINATIONS {
            return Err(RunContractError::TooManyDestinations);
        }
        Ok(Self { destinations })
    }
}
