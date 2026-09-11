//! `--allow <scopes>` parsing: the CLI side of the run approval.
//!
//! A headless run cannot prompt, so its capability set must be stated before
//! anything starts — and it must be stated exactly: an unknown scope name is
//! a usage error, never a silently ignored token (a run that believes it
//! approved more or less than the user typed is a lying run). The grammar
//! mirrors the [`Capabilities`] contract fields; the built approval is
//! exactly the stated scopes, nothing implicit.

use saya_types::{Capabilities, Destination, EndpointBindings, FetchScope, RunnerScope};

/// The scope grammar, for the error message that names what was refused.
const KNOWN: &str = "known scopes: workspace-write, scratch, fetch:<scheme>+<host>, \
                     runner:<program>, endpoint:<role>=<endpoint>";

/// The scopes `--allow` approved.
pub(super) struct Approved {
    pub(super) capabilities: Capabilities,
}

impl Approved {
    /// True when nothing at all was approved — the shape the
    /// refusal-by-construction rule checks before anything starts.
    pub(super) fn is_empty(&self) -> bool {
        let caps = &self.capabilities;
        !caps.workspace_write
            && !caps.scratch
            && caps.fetch.is_none()
            && caps.runner.is_none()
            && caps.endpoints.as_map().is_empty()
    }
}

/// Parses the `--allow` tokens. An empty list is the caller's refusal
/// decision, not a silently-empty approval; anything here that does not
/// match the grammar is a typed usage error.
pub(super) fn parse(tokens: &[String]) -> Result<Approved, String> {
    let mut capabilities = Capabilities::default();
    let mut destinations = Vec::new();
    let mut programs = Vec::new();
    let mut bindings = Vec::new();
    for token in tokens {
        if token == "workspace-write" {
            capabilities.workspace_write = true;
        } else if token == "scratch" {
            capabilities.scratch = true;
        } else if let Some(rest) = token.strip_prefix("fetch:") {
            let Some((scheme, host)) = rest.split_once('+') else {
                return Err(format!(
                    "scope `{token}` must be fetch:<scheme>+<host>; {KNOWN}"
                ));
            };
            let destination = Destination::new(scheme, host).map_err(|_| {
                format!("scope `{token}` is not a scheme plus a bare host; {KNOWN}")
            })?;
            destinations.push(destination);
        } else if let Some(rest) = token.strip_prefix("runner:") {
            programs.push(rest.to_string());
        } else if let Some(rest) = token.strip_prefix("endpoint:") {
            let Some((role, endpoint)) = rest.split_once('=') else {
                return Err(format!(
                    "scope `{token}` must be endpoint:<role>=<endpoint>; {KNOWN}"
                ));
            };
            bindings.push((role.to_string(), endpoint.to_string()));
        } else {
            return Err(format!("unknown scope `{token}`; {KNOWN}"));
        }
    }
    if !destinations.is_empty() {
        capabilities.fetch = Some(
            FetchScope::new(destinations)
                .map_err(|error| format!("fetch scope refused: {error}"))?,
        );
    }
    if !programs.is_empty() {
        capabilities.runner = Some(
            RunnerScope::new(programs).map_err(|error| format!("runner scope refused: {error}"))?,
        );
    }
    if !bindings.is_empty() {
        capabilities.endpoints = EndpointBindings::new(bindings)
            .map_err(|error| format!("endpoint bindings refused: {error}"))?;
    }
    Ok(Approved { capabilities })
}
