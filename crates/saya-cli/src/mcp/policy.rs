//! Bounds and the startup allowlist for `saya mcp serve` (ADR 0008, task Da).
//!
//! The server speaks newline-delimited JSON-RPC over stdio; everything a
//! client can make it touch is bounded here. The data tools of the follow-on
//! database wiring (task Db) plug into the same hooks; the skeleton exercises
//! them on the meta tool.

use std::time::Duration;

use crate::config::runtime::RuntimeConfig;

/// Largest accepted request payload, in bytes (1 MiB).
///
/// Enforced where the handler sees a request: a `tools/call` whose arguments
/// exceed it is refused before dispatch. The raw line is decoded by rmcp's
/// stdio transport, which has no inbound line cap of its own; the data tools
/// (task Db) move this gate onto the request line at their dispatch boundary.
pub(crate) const MAX_REQUEST_BYTES: usize = 1_048_576;

/// Tool calls served concurrently; a call beyond the cap is refused, never
/// queued.
pub(crate) const MAX_IN_FLIGHT: usize = 4;

/// Wall-clock ceiling per tool call.
pub(crate) const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// Largest accepted serialized tool response, in bytes (16 MiB).
pub(crate) const MAX_RESPONSE_BYTES: usize = 16 * 1_048_576;

/// One entry of the startup allowlist: the profile's name and SQL dialect —
/// never paths, hosts, or identities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProfileSummary {
    pub(crate) name: String,
    pub(crate) dialect: &'static str,
}

/// What this server may serve, decided once at startup and never widened by
/// a client: the profile allowlist, the data-sharing gate, and the call
/// bounds.
#[derive(Debug, Clone)]
pub(crate) struct ServePolicy {
    allowlist: Vec<ProfileSummary>,
    allow_data_sharing: bool,
}

impl ServePolicy {
    /// The profile allowlist: the serve-local `--profile` values, else the
    /// pre-subcommand `--profile`, else the configured default profile, else
    /// none. A name that does not resolve to a configured profile is refused:
    /// the server never starts on an unnamed intent.
    ///
    /// No entry here can grant anything later — `tools/list` and every future
    /// data tool read exactly this set.
    pub(crate) fn resolve(
        runtime: &RuntimeConfig,
        profiles: &[String],
        cli_profile: Option<&str>,
    ) -> Result<Self, String> {
        let connections = &runtime.connections.profiles;
        let mut names: Vec<String> = Vec::new();
        if profiles.is_empty() {
            if let Some(name) = cli_profile {
                names.push(name.to_owned());
            } else if let Some(name) = runtime.resolved.profile_name.as_ref() {
                names.push(name.clone());
            }
        } else {
            for name in profiles {
                if !names.contains(name) {
                    names.push(name.clone());
                }
            }
        }
        for name in &names {
            if !connections.contains_key(name) {
                return Err(format!("connection profile {name:?} was not found"));
            }
        }
        Ok(Self {
            allowlist: names
                .into_iter()
                .map(|name| ProfileSummary {
                    dialect: connections[&name].dialect().as_str(),
                    name,
                })
                .collect(),
            allow_data_sharing: runtime.resolved.ai.allow_data_sharing,
        })
    }

    pub(crate) fn allowlist(&self) -> &[ProfileSummary] {
        &self.allowlist
    }

    /// Whether row-returning tools may be listed at all (task Db reads this;
    /// the flag/config fold is resolve's, not a second one).
    pub(crate) const fn allow_data_sharing(&self) -> bool {
        self.allow_data_sharing
    }

    pub(crate) const fn call_timeout(&self) -> Duration {
        CALL_TIMEOUT
    }

    pub(crate) const fn max_in_flight(&self) -> usize {
        MAX_IN_FLIGHT
    }

    /// Whether a request payload of this size is accepted.
    pub(crate) const fn request_allowed(&self, bytes: usize) -> bool {
        bytes <= MAX_REQUEST_BYTES
    }

    /// Whether a serialized response of this size may be sent.
    pub(crate) const fn response_allowed(&self, bytes: usize) -> bool {
        bytes <= MAX_RESPONSE_BYTES
    }
}
