//! The session's host-command lane composition, once per session process:
//! the unsandboxed second lane's own opt-in. The lane is off unless stated
//! at launch — `--host-commands`, a `--allow command:<x>` seed (which
//! implies composition), or user-layer `[host_commands] enable` — and it
//! never composes without a bound workspace root: no root, no lane, even
//! with the flag. A project-layer `[host_commands]` is a typed resolve
//! error before this module ever runs (see `saya-config`), because a
//! model-writable file must never enable unsandboxed execution.
//!
//! The lane is **not contained**: the child runs as the user's uid with the
//! whole filesystem and network, resolved on the user's PATH. Nothing here
//! claims a bound the code does not apply — H0's module header is the
//! register this module matches.

use std::path::PathBuf;

/// What the launch stated about the lane: the flag, the `--allow` seeds,
/// the `--deny` refusals, and the user-layer config — read together, once,
/// at composition.
pub(crate) struct HostLaunch {
    flag: bool,
    seeds: Vec<String>,
    deny: Vec<String>,
    config_enabled: bool,
    config: saya_config::ResolvedHostCommands,
}

impl HostLaunch {
    /// Reads the launch statement: the `--host-commands` flag, the session's
    /// `--allow` seeds and `--deny` refusals, and the resolved user-layer
    /// `[host_commands]`.
    pub(crate) fn from_options(
        options: &crate::cli::GlobalOptions,
        runtime: &RuntimeConfig,
    ) -> Self {
        Self {
            flag: options.host_commands,
            seeds: options.allow.clone(),
            deny: options.deny.clone(),
            config_enabled: runtime.resolved.host_commands.enabled,
            config: runtime.resolved.host_commands.clone(),
        }
    }

    /// True when the launch stated the lane: the flag, any `command:` seed
    /// (the seed implies composition), or user-layer `enable`.
    pub(crate) fn composes_lane(&self) -> bool {
        self.flag
            || self.config_enabled
            || self.seeds.iter().any(|seed| seed.starts_with("command:"))
    }

    /// The `command:` seeds the launch stated, verbatim and in order.
    pub(crate) fn command_seeds(&self) -> Vec<String> {
        self.seeds
            .iter()
            .filter(|seed| seed.starts_with("command:"))
            .cloned()
            .collect()
    }

    /// The test seam: a stated lane (flag-equivalent) over the runtime's own
    /// resolved `[host_commands]`.
    #[cfg(test)]
    pub(crate) fn for_tests_stated(runtime: &RuntimeConfig) -> Self {
        Self {
            flag: true,
            seeds: Vec::new(),
            deny: Vec::new(),
            config_enabled: runtime.resolved.host_commands.enabled,
            config: runtime.resolved.host_commands.clone(),
        }
    }

    /// The test seam: an unstated lane carrying only `--deny` refusals —
    /// refusal-only, composes nothing.
    #[cfg(test)]
    pub(crate) fn from_deny_for_tests(deny: Vec<String>) -> Self {
        Self {
            flag: false,
            seeds: Vec::new(),
            deny,
            config_enabled: false,
            config: saya_config::ResolvedHostCommands::default(),
        }
    }

    /// The launch's `--deny` refusals, verbatim and in order.
    pub(crate) fn deny_list(&self) -> Vec<String> {
        self.deny.clone()
    }

    /// Seeds the launch's `command:` tokens into the provided grant store:
    /// the grammar stays the authority (each token parses on the session
    /// surface). The session loop seeds through
    /// `session_grants::seed_launch_allow` instead (which journals); this
    /// stays as the launch helper's unit — exercised by the host tests
    /// below — so the seed-implies-composition half has a direct pin.
    pub(crate) fn seed_grants(
        &self,
        grants: &saya_agent::SessionGrants,
    ) -> Result<Vec<String>, String> {
        let seeds = self.command_seeds();
        for seed in &seeds {
            crate::commands::run::scopes::parse(
                std::slice::from_ref(seed),
                crate::commands::run::scopes::Surface::Session,
            )?;
            grants.grant(seed);
        }
        Ok(seeds)
    }
}

use crate::config::runtime::RuntimeConfig;

/// What composing the lane produced: the executor config plus the facts the
/// prompts and `/allow` consult — or nothing, when the launch did not state
/// the lane or no workspace root bound.
pub(crate) struct SessionHost {
    /// The executor configuration: the child's PATH, ceiling, and passed
    /// variables — built once, shared across calls.
    pub(crate) config: saya_harness::host::HostConfig,
    /// The facts `/allow command:<x>` consults and the prompt states.
    pub(crate) facts: crate::approval_facts::HostFacts,
}

/// Composes the lane once: stated at launch (or in the user layer) **and** a
/// workspace root bound — no root, no lane, even with the flag. `path_value`
/// is the exact PATH the child receives; under Windows the lane fails
/// closed (the group-kill core is unix-verified, the runner's posture).
pub(crate) fn compose_host(
    launch: &HostLaunch,
    workspace_root: Option<&std::path::Path>,
    path_value: String,
) -> Result<Option<SessionHost>, String> {
    if !launch.composes_lane() {
        return Ok(None);
    }
    let Some(root) = workspace_root else {
        return Ok(None);
    };
    if cfg!(windows) {
        return Err(
            "host commands are unavailable on Windows: the process-group kill is \
                    unix-verified (the runner's posture), so the lane fails closed"
                .to_owned(),
        );
    }
    let timeout = std::time::Duration::from_secs(launch.config.timeout_seconds);
    let mut config = saya_harness::host::HostConfig::new(path_value, timeout)
        .map_err(|error| format!("the host-command lane could not be composed: {error}"))?;
    let mut pass_env = Vec::new();
    for name in &launch.config.pass_env {
        let value = std::env::var(name).map_err(|_| {
            format!("host command refused: `pass_env` names `{name}`, which is not set")
        })?;
        pass_env.push((name.clone(), value));
    }
    config = config
        .with_extra_env(pass_env)
        .map_err(|error| format!("the host-command lane could not be composed: {error}"))?;
    Ok(Some(SessionHost {
        config,
        facts: crate::approval_facts::HostFacts {
            workspace_root: root.to_path_buf(),
            timeout_seconds: launch.config.timeout_seconds,
            pass_env: launch.config.pass_env.clone(),
        },
    }))
}

/// The launch's own words about what it stated: the flag, the seeds, and the
/// user-layer enable — for the session-startup notice. Nothing secret rides
/// it: `pass_env` names travel, values never do.
pub(crate) fn launch_notice(launch: &HostLaunch, _root: Option<PathBuf>) -> Option<String> {
    if !launch.composes_lane() {
        return None;
    }
    let mut parts = vec!["host commands: unsandboxed lane stated".to_owned()];
    if launch.flag {
        parts.push("`--host-commands`".to_owned());
    }
    let seeds = launch.command_seeds();
    if !seeds.is_empty() {
        parts.push(format!("seeded: {}", seeds.join(", ")));
    }
    if launch.config_enabled {
        parts.push("user `[host_commands] enable`".to_owned());
    }
    Some(parts.join(" — "))
}
