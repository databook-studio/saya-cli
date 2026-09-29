//! `saya mcp serve` (ADR 0008, task Da): a stdio MCP server skeleton.
//!
//! [`policy`] holds the bounds and the startup allowlist; [`server`] is the
//! rmcp glue. The data tools of the database wiring (task Db) extend the
//! server behind the policy hooks. stdout carries protocol frames only; logs
//! go to stderr.

mod policy;
mod server;
mod tools;

#[cfg(test)]
mod mcp_tests;

use std::path::Path;

use crate::{cli::GlobalOptions, config};

/// Run `saya mcp serve` to completion and return the process exit code.
///
/// Config resolution is the same `config::runtime::load` every subcommand
/// runs. The profile allowlist is the serve-local `--profile` values, else
/// the pre-subcommand `--profile`, else the configured default profile, else
/// none; the first serve-local value also seeds the active-profile slot so a
/// multi-profile allowlist resolves on a config with no default, and any
/// unknown name is refused by the same resolve path every other subcommand
/// uses.
pub(crate) fn serve(
    options: &GlobalOptions,
    profiles: Vec<String>,
) -> Result<i32, Box<dyn std::error::Error>> {
    let mut options = options.clone();
    if options.profile.is_none()
        && let Some(first) = profiles.first()
    {
        options.profile = Some(first.clone());
    }
    let runtime = config::runtime::load(&options, Path::new("."))?;
    let policy = policy::ServePolicy::resolve(&runtime, &profiles, options.profile.as_deref())?;
    let allowlist = policy
        .allowlist()
        .iter()
        .map(|profile| format!("{} ({})", profile.name, profile.dialect))
        .collect::<Vec<_>>()
        .join(", ");
    eprintln!(
        "saya mcp: profiles: {}; data sharing: {}",
        if allowlist.is_empty() {
            "(none)".to_owned()
        } else {
            allowlist
        },
        if policy.allow_data_sharing() {
            "allowed"
        } else {
            "off"
        },
    );
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(server::run(policy))
}
