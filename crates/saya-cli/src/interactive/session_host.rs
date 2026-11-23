//! The session's host-command lane composition, once per session process:
//! the unsandboxed second lane, composed wherever a workspace root binds.
//! The lane needs no declaration: the per-call ask is the gate under `ask`,
//! and choosing `--approval-mode bypass` is itself the deliberate act, so
//! the lane composes with no root-bound session unstated. It never composes
//! without a bound workspace root: no root, no lane — structural, not
//! ceremony, because the child's cwd is pinned to the root. A
//! project-layer `[host_commands]` is a typed resolve error before this
//! module ever runs (see `saya-config`), because a model-writable file must
//! never shape unsandboxed execution.
//!
//! The lane is **not contained**: the child runs as the user's uid with the
//! whole filesystem and network, resolved on the user's PATH. Nothing here
//! claims a bound the code does not apply.

/// What the launch stated about the lane: the `--deny` refusals and the
/// user-layer config — read together, once, at composition. Nothing here
/// decides whether the lane composes: a bound root does. Launch `--allow`
/// seeds are handled by the session grant flow after composition.
pub(crate) struct HostLaunch {
    deny: Vec<String>,
    config: saya_config::ResolvedHostCommands,
}

impl HostLaunch {
    /// Reads the launch statement: the session's `--deny` refusals and the
    /// resolved user-layer `[host_commands]`.
    pub(crate) fn from_options(
        options: &crate::cli::GlobalOptions,
        runtime: &RuntimeConfig,
    ) -> Self {
        Self {
            deny: options.deny.clone(),
            config: runtime.resolved.host_commands.clone(),
        }
    }

    /// A launch with no statement: no refusals, over the runtime's own
    /// resolved `[host_commands]` shaping. The lane still composes wherever
    /// a root binds — unstated is not off.
    pub(crate) fn unstated(runtime: &RuntimeConfig) -> Self {
        Self {
            deny: Vec::new(),
            config: runtime.resolved.host_commands.clone(),
        }
    }

    /// The test seam: a launch over the runtime's own resolved
    /// `[host_commands]`.
    #[cfg(test)]
    pub(crate) fn for_tests_stated(runtime: &RuntimeConfig) -> Self {
        Self::from_options(
            &crate::cli::GlobalOptions {
                deny: Vec::new(),
                ..Default::default()
            },
            runtime,
        )
    }

    /// The test seam: a launch carrying only `--deny` refusals —
    /// refusal-only, composes nothing on its own.
    #[cfg(test)]
    pub(crate) fn from_deny_for_tests(deny: Vec<String>) -> Self {
        Self {
            deny,
            config: saya_config::ResolvedHostCommands::default(),
        }
    }

    /// The launch's `--deny` refusals, verbatim and in order.
    pub(crate) fn deny_list(&self) -> Vec<String> {
        self.deny.clone()
    }
}

use crate::config::runtime::RuntimeConfig;

/// The no-PATH fact, said on the composition notice seam at startup: the
/// lane needs the parent's PATH to resolve the child's programs, so without
/// one the lane composes nothing — and the session continues without it.
/// The register matches the unbound-workspace notice: the fact, why the
/// tool is absent, and the remedy.
pub(crate) const NO_PATH_NOTICE: &str = "No PATH is set, so the host-command lane is not composed: \
    run_command is unavailable; set PATH to reach host programs.";

/// Composes the lane from an explicit PATH value: `None` — no PATH in the
/// environment — composes the lane away with the no-PATH notice, never an
/// error. A missing PATH disables the lane, not the session: cron jobs,
/// systemd units, and minimal containers start fine, minus `run_command`.
/// `Some` composes through [`compose_host`] unchanged.
pub(crate) fn compose_host_lane(
    launch: &HostLaunch,
    workspace_root: Option<&std::path::Path>,
    path: Option<String>,
) -> Result<(Option<SessionHost>, Option<String>), String> {
    let Some(root) = workspace_root else {
        return Ok((None, None));
    };
    let Some(path_value) = effective_path(path) else {
        return Ok((None, Some(NO_PATH_NOTICE.to_owned())));
    };
    Ok((compose_host(launch, Some(root), path_value)?, None))
}

/// The PATH the lane searches. Unix uses the variable as given, and its
/// absence is the no-PATH notice. Windows always has a system-defined search
/// path — the process and system directories — even when the variable is
/// missing, so the lane composes over that rather than reporting a lane
/// Windows cannot actually be missing.
fn effective_path(path: Option<String>) -> Option<String> {
    #[cfg(windows)]
    {
        Some(path.unwrap_or_else(saya_harness::proc::default_search_path))
    }
    #[cfg(not(windows))]
    {
        path
    }
}

/// What composing the lane produced: the executor config plus the facts the
/// prompts and `/allow` consult — or nothing, when no workspace root bound
/// or no PATH is set (the no-PATH seam above composes the lane away).
pub(crate) struct SessionHost {
    /// The facts `/allow command:<x>` consults and the prompt states.
    pub(crate) facts: crate::approval_facts::HostFacts,
}

/// Composes the lane once: wherever a workspace root binds — no root, no
/// lane, structural, because the child's cwd is pinned to the root.
/// `path_value` is the exact PATH the child receives. The lane is
/// unsandboxed on every platform: on Windows the process-tree kill is the
/// Job-object/`taskkill` path, and nothing there claims an OS confinement.
pub(crate) fn compose_host(
    launch: &HostLaunch,
    workspace_root: Option<&std::path::Path>,
    path_value: String,
) -> Result<Option<SessionHost>, String> {
    let Some(root) = workspace_root else {
        return Ok(None);
    };
    let timeout = std::time::Duration::from_secs(launch.config.timeout_seconds);
    let config = saya_harness::host::HostConfig::new(path_value, root.to_path_buf(), timeout)
        .map_err(|error| format!("the host-command lane could not be composed: {error}"))?;
    let mut pass_env = Vec::new();
    for name in &launch.config.pass_env {
        let value = std::env::var(name).map_err(|_| {
            format!("host command refused: `pass_env` names `{name}`, which is not set")
        })?;
        pass_env.push((name.clone(), value));
    }
    // The composed config is validated here and dropped: the executor that
    // spawned through it went with the deleted turn body, so only the facts
    // (what `/allow command:<x>` consults, and what the prompt states) are
    // kept — while a PATH, ceiling or unset `pass_env` still refuses the lane
    // at composition, never at the first call.
    let _ = config
        .with_extra_env(pass_env)
        .map_err(|error| format!("the host-command lane could not be composed: {error}"))?;
    Ok(Some(SessionHost {
        facts: crate::approval_facts::HostFacts {
            workspace_root: root.to_path_buf(),
            timeout_seconds: launch.config.timeout_seconds,
            pass_env: launch.config.pass_env.clone(),
        },
    }))
}
