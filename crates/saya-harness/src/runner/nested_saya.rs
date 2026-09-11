//! The nested re-entry (M5-5): when a run re-enters saya as a child — the
//! `bench/spider` question shape (`bench/spider/bench.py:321-332`), now
//! engine-supervised — the child's world is the run's own state and nothing
//! else. Three closures make that true, and this module composes all of
//! them:
//!
//! - **The generated config** — [`crate::endpoints::write_run_configs`],
//!   whose first production caller is [`prepare`] here — writes
//!   `state/config/{config,connections}.toml` with references only
//!   (`api_key = { env = "SAYA_RUN_EP_<ROLE>" }`); the resolved values
//!   reach the child through the spawn environment, never disk.
//! - **The pinned environment** — `SAYA_CONFIG_HOME`, `SAYA_STATE_DB`, and
//!   `SAYA_SESSION_DIR` all point inside `runs/<id>/state/` (consumed by
//!   the child at `saya-cli/src/config/sources.rs:29`, `state_path.rs:7`,
//!   and `interactive/session_paths.rs:16`), so the child's own fallback
//!   resolution lands in run state. The spawn environment is built, not
//!   inherited, so these three pins are the whole of what the child's
//!   resolution can see.
//! - **The pinned cwd** — the proven spawn pins the child's cwd to the
//!   first filesystem root, so the caller's policy must list the run
//!   workspace first (with the run's `state/` beside it). Project
//!   `connections.toml` is auto-discovered from the current directory
//!   (`saya-cli/src/config/sources.rs:17-25`); pinning the cwd closes that
//!   hole from the inside while the sandbox denies reads outside the roots
//!   and closes it from the outside — both, not either.
//!
//! The invocation is spawned only through
//! [`RunnerSpawn`](super::sandbox::RunnerSpawn) — the startup probe's
//! verdict, consumed, never re-derived — after the same refusal battery
//! (`refuse::validate_call`) every `run_program` call passes: the nested
//! program is a bare, allowlisted, staged, non-script binary.
//!
//! One flag the bench does not pass: `--trust-project-config`. An explicit
//! `--config` reaches the child's resolution as its *project* layer, and an
//! untrusted project layer's wholesale endpoints are reverted — the
//! engine's generated endpoints would vanish. The generated file is
//! authored by this composition root itself (0600, inside the run dir,
//! references only), so the trust the flag declares is already structural;
//! and pinning `SAYA_CONFIG_HOME` inside run state keeps the child's user
//! layer empty — that emptiness is the production-profile absence (DESIGN
//! §6.4), so the bench's trick of pointing both config layers at one file
//! is not available here.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::Duration,
};

use saya_agent::CancellationToken;
use saya_types::{EndpointBindings, RunnerScope};

use crate::{
    endpoints::{
        CorpusProfile, EndpointSpec, ORCHESTRATOR_ROLE, RunConfigError, endpoint_env_var,
        write_run_configs,
    },
    run_dir::RunDir,
};

use super::{
    RunnerError,
    env::{Credential, CredentialSource},
    output::ProgramOutcome,
    refuse::validate_call,
    sandbox::RunnerSpawn,
    spawn,
};

/// The program a nested re-entry runs: the `saya` binary itself, staged in
/// the runner's program directory and allowlisted by the step's scope. A
/// bare name like every runner program, and not an interpreter, so the
/// refusal battery admits it.
pub const NESTED_PROGRAM: &str = "saya";

/// The environment variables the nested child's own resolution reads. Each
/// is pinned inside `runs/<id>/state/` — the child's user config, state
/// database, and session directory are run state, never the operator's.
const CONFIG_HOME_VAR: &str = "SAYA_CONFIG_HOME";
const STATE_DB_VAR: &str = "SAYA_STATE_DB";
const SESSION_DIR_VAR: &str = "SAYA_SESSION_DIR";

/// The nested child's pinned world: where its generated config lives, and
/// the paths its own resolution reads — all inside `runs/<id>/state/`.
/// Built by [`prepare`], consumed by [`NestedState::ask_argv`] and [`ask`].
pub struct NestedState {
    state: PathBuf,
}

impl NestedState {
    /// The run's state directory every pinned path lives inside
    /// (`runs/<id>/state/`).
    pub fn state_dir(&self) -> &std::path::Path {
        &self.state
    }

    /// The generated `config.toml` the child is launched against.
    pub fn config_toml(&self) -> PathBuf {
        self.state.join("config/config.toml")
    }

    /// The generated `connections.toml` — the run's corpus, and nothing else.
    pub fn connections_toml(&self) -> PathBuf {
        self.state.join("config/connections.toml")
    }

    /// The three pins, in the order they are set on the child's command;
    /// every path is inside `runs/<id>/state/`.
    pub fn pinned_env(&self) -> [(&'static str, PathBuf); 3] {
        [
            (CONFIG_HOME_VAR, self.state.join("config-home")),
            (STATE_DB_VAR, self.state.join("state.sqlite3")),
            (SESSION_DIR_VAR, self.state.join("sessions")),
        ]
    }
    /// The exact `saya ask` invocation the run re-enters with — the
    /// `bench/spider` question shape (`bench.py:321-332`), flag order
    /// pinned, plus `--trust-project-config` (see the module docs).
    pub fn ask_argv(&self, profile: &str, question: &str) -> Vec<String> {
        vec![
            "ask".into(),
            "--config".into(),
            self.config_toml().display().to_string(),
            "--connections".into(),
            self.connections_toml().display().to_string(),
            "--profile".into(),
            profile.to_owned(),
            "--non-interactive".into(),
            "--trust-project-config".into(),
            "--approval-mode".into(),
            "read-only".into(),
            "--format".into(),
            "ndjson".into(),
            question.to_owned(),
        ]
    }
}

/// Generates the run's child configuration and returns the pinned world the
/// nested child launches into.
pub fn prepare(
    run: &RunDir,
    endpoints: &[EndpointSpec],
    bindings: &EndpointBindings,
    corpus: &[CorpusProfile],
) -> Result<NestedState, RunConfigError> {
    write_run_configs(run, endpoints, bindings, corpus)?;
    Ok(NestedState {
        state: run.state().to_path_buf(),
    })
}

/// The declared credentials whose resolved values ride the spawn
/// environment into the child: one per role the generated config binds
/// (`write_run_configs`' role set, the bound roles plus
/// [`ORCHESTRATOR_ROLE`]), under the exact env var the generated
/// `config.toml` references — a reference, resolved at spawn time.
pub fn endpoint_credentials(
    endpoints: &[EndpointSpec],
    bindings: &EndpointBindings,
) -> Result<Vec<Credential>, RunnerError> {
    let mut pool = BTreeMap::new();
    for endpoint in endpoints {
        if pool.insert(endpoint.name.as_str(), endpoint).is_some() {
            return Err(RunnerError::CredentialInvalid {
                credential: endpoint.name.clone(),
                reason: "the run's endpoint pool declares this name twice",
            });
        }
    }
    let mut roles: BTreeSet<&str> = bindings.as_map().keys().map(String::as_str).collect();
    roles.insert(ORCHESTRATOR_ROLE);
    let mut credentials = Vec::new();
    let mut env_names = BTreeSet::new();
    for role in &roles {
        let endpoint = bindings.get(role).unwrap_or(ORCHESTRATOR_ROLE);
        let Some(spec) = pool.get(endpoint) else {
            return Err(RunnerError::CredentialInvalid {
                credential: endpoint.to_string(),
                reason: "the bound endpoint is not in the run's endpoint pool",
            });
        };
        let Some(reference) = &spec.api_key else {
            continue;
        };
        let env_var = endpoint_env_var(role);
        if !env_names.insert(env_var.clone()) {
            return Err(RunnerError::CredentialInvalid {
                credential: env_var,
                reason: "two roles resolve to the same environment binding",
            });
        }
        credentials.push(Credential::new(
            spec.name.clone(),
            env_var,
            reference.clone(),
        )?);
    }
    Ok(credentials)
}

/// One nested ask, ready to spawn: everything that varies per question —
/// the profile the child answers as, the question, and the step's declared
/// credentials with their resolver.
pub struct NestedAsk<'a> {
    pub profile: &'a str,
    pub question: &'a str,
    pub credentials: &'a [Credential],
    pub resolver: &'a dyn CredentialSource,
}

/// Spawns the nested ask: the same refusal battery every `run_program` call
/// passes, then the proven spawn with the pinned environment. The child's
/// cwd is the run workspace — the proven spawn pins the cwd to the first
/// filesystem root, so the caller's policy must list the run workspace
/// first.
pub async fn ask(
    spawn: &RunnerSpawn,
    scope: &RunnerScope,
    state: &NestedState,
    ask: NestedAsk<'_>,
    default_timeout: Duration,
    cancellation: &CancellationToken,
) -> Result<ProgramOutcome, RunnerError> {
    let argv = state.ask_argv(ask.profile, ask.question);
    let call = validate_call(
        scope,
        spawn.program_dir(),
        default_timeout,
        None,
        NESTED_PROGRAM,
        &argv,
    )?;
    let pinned = state.pinned_env();
    spawn::run(
        spawn,
        call,
        &pinned,
        ask.credentials,
        ask.resolver,
        cancellation,
    )
    .await
}
