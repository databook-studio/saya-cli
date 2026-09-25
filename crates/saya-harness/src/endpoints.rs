//! Generated run configuration: `state/config/{config,connections}.toml`,
//! the files a run's child processes read at launch.
//!
//! The one rule: **a resolved secret value never reaches disk.** Every
//! `api_key` written here is a reference into the spawn environment —
//! `api_key = { env = "SAYA_RUN_EP_<ROLE>" }` — and the resolved value
//! lives only in the child process's memory. The original `SecretRef`
//! never lands in these files either, only its presence does; production
//! profiles are absent, not denied (DESIGN §6.4): a denied-but-present
//! profile is a lock someone can pick, an absent one is not there to pick.
//!
//! The input types are a deliberate, minimal mirror of `saya-config`'s
//! resolved shapes: a `saya-harness → saya-config` dependency would drag
//! config machinery into the run engine's every API, and the translation
//! belongs at the composition root (`saya-cli`). Do not "fix" the mirror
//! by adding that dependency.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path},
};

use saya_types::{EndpointBindings, MAX_ENDPOINT_BINDINGS, SecretRef, is_name_shaped};
use thiserror::Error;

use crate::{HarnessError, io_error, run_dir::RunDir};

/// The role every run has; a mirror of `saya-config::ORCHESTRATOR_ROLE`
/// (see the module docs) — the pool's entry of this name is the fallback.
pub const ORCHESTRATOR_ROLE: &str = "orchestrator";

/// One endpoint of the run's pool as the composition root resolves it —
/// a deliberate mirror of `saya-config::ResolvedEndpoint`, provider as a string.
#[derive(Debug, Clone, PartialEq)]
pub struct EndpointSpec {
    /// The endpoint's run-scoped name — the key roles bind to.
    pub name: String,
    pub provider: String,
    pub model: String,
    pub base_url: Option<String>,
    /// A reference, never a resolved value. Only its presence is read: a
    /// present key binds `{ env = "SAYA_RUN_EP_<ROLE>" }` in the generated
    /// `config.toml`; the value lives only in the child's environment.
    pub api_key: Option<SecretRef>,
}

/// One corpus database staged for this run: the profile name it answers to
/// and its copy's path relative to `runs/<id>/`. The caller passes what it
/// staged; nothing here reads the user's real `connections.toml`.
#[derive(Debug, Clone, PartialEq)]
pub struct CorpusProfile {
    pub name: String,
    pub path: String,
}

/// Failures of generated run configuration; rendered by `saya-cli`.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RunConfigError {
    #[error("role {role} binds endpoint {endpoint:?}, which is not in the run's endpoint pool")]
    UnknownEndpoint { role: String, endpoint: String },

    #[error("the run's endpoint pool declares {name:?} twice")]
    DuplicateEndpoint { name: String },

    #[error("name is not a valid run-scoped name: {name:?}")]
    InvalidName { name: String },

    #[error("corpus path must stay inside the run directory: {path:?}")]
    CorpusPathOutsideRun { path: String },

    #[error("two roles resolve to the same endpoint env binding: {var}")]
    DuplicateEnvBinding { var: String },

    #[error("the run's endpoint pool holds {found} entries, max {max}")]
    TooManyEndpoints { found: usize, max: usize },

    #[error(transparent)]
    Harness(#[from] HarnessError),
}

/// Resolves the run's roles to endpoints and writes `state/config/`:
/// `config.toml`, one `[[endpoint]]` table per role, and `connections.toml`
/// with the staged corpus and nothing else. Roles written are the run's
/// bound roles plus [`ORCHESTRATOR_ROLE`]: a bound role is served by the
/// endpoint it names, an unbound one by the pool's `orchestrator` entry.
/// Every generated `api_key` is a reference into the spawn environment;
/// rewrites in place on resume and repairs file modes to 0600.
pub fn write_run_configs(
    run: &RunDir,
    endpoints: &[EndpointSpec],
    bindings: &EndpointBindings,
    corpus: &[CorpusProfile],
) -> Result<(), RunConfigError> {
    if endpoints.len() > MAX_ENDPOINT_BINDINGS {
        return Err(RunConfigError::TooManyEndpoints {
            found: endpoints.len(),
            max: MAX_ENDPOINT_BINDINGS,
        });
    }
    let mut pool = BTreeMap::new();
    for endpoint in endpoints {
        check_name(&endpoint.name)?;
        if pool.insert(endpoint.name.as_str(), endpoint).is_some() {
            return Err(RunConfigError::DuplicateEndpoint {
                name: endpoint.name.clone(),
            });
        }
    }

    let mut roles: BTreeSet<&str> = bindings.as_map().keys().map(String::as_str).collect();
    roles.insert(ORCHESTRATOR_ROLE);
    let mut resolved = Vec::with_capacity(roles.len());
    let mut env_names = BTreeSet::new();
    for role in &roles {
        let endpoint = bindings.get(role).unwrap_or(ORCHESTRATOR_ROLE);
        let spec = pool
            .get(endpoint)
            .ok_or_else(|| RunConfigError::UnknownEndpoint {
                role: (*role).to_string(),
                endpoint: endpoint.to_string(),
            })?;
        let env = endpoint_env_var(role);
        if !env_names.insert(env.clone()) {
            return Err(RunConfigError::DuplicateEnvBinding { var: env });
        }
        resolved.push((*role, *spec, env));
    }

    for profile in corpus {
        check_name(&profile.name)?;
        inside_run_dir(&profile.path)
            .map_err(|path| RunConfigError::CorpusPathOutsideRun { path })?;
    }

    let dir = run.state().join("config");
    fs::create_dir_all(&dir).map_err(|error| io_error("create run config dir", &dir, error))?;
    #[cfg(unix)]
    set_mode(&dir, 0o700)?;
    write_private(&dir.join("config.toml"), &render_config(&resolved))?;
    write_private(&dir.join("connections.toml"), &render_connections(corpus))?;
    Ok(())
}

/// The env var a role's resolved key is injected under at launch, built
/// under `saya_types::CREDENTIAL_ENV_PREFIX` — the constant `redact()` matches.
pub fn endpoint_env_var(role: &str) -> String {
    let mut var = String::from(saya_types::CREDENTIAL_ENV_PREFIX);
    var.extend(role.chars().map(|c| match c {
        c if c.is_ascii_alphanumeric() => c.to_ascii_uppercase(),
        _ => '_',
    }));
    var
}

fn check_name(name: &str) -> Result<(), RunConfigError> {
    if is_name_shaped(name) {
        return Ok(());
    }
    Err(RunConfigError::InvalidName {
        name: name.to_string(),
    })
}

/// True when `path` is a clean relative path: every component normal.
fn inside_run_dir(path: &str) -> Result<(), String> {
    let clean = !path.is_empty()
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)));
    clean.then_some(()).ok_or_else(|| path.to_string())
}

fn render_config(resolved: &[(&str, &EndpointSpec, String)]) -> String {
    let mut out = String::new();
    out.push_str("# Generated run configuration. Rewritten on resume; do not edit.\n");
    out.push_str("# api_key binds a reference into the spawn environment. A resolved\n");
    out.push_str("# secret value is held in child memory only and never written here.\n");
    for (role, spec, env) in resolved {
        out.push_str("\n[[endpoint]]\n");
        out.push_str(&format!("role = {}\n", toml_string(role)));
        out.push_str(&format!("provider = {}\n", toml_string(&spec.provider)));
        out.push_str(&format!("model = {}\n", toml_string(&spec.model)));
        if let Some(base_url) = &spec.base_url {
            out.push_str(&format!("base_url = {}\n", toml_string(base_url)));
        }
        if spec.api_key.is_some() {
            out.push_str(&format!("api_key = {{ env = {} }}\n", toml_string(env)));
        }
    }
    out
}

fn render_connections(corpus: &[CorpusProfile]) -> String {
    let mut out = String::new();
    out.push_str("# Generated run corpus. Only this run's staged databases appear here.\n");
    out.push_str("# Production profiles are absent by design, not denied (DESIGN §6.4).\n");
    for profile in corpus {
        out.push_str(&format!("\n[profiles.{}]\n", toml_string(&profile.name)));
        out.push_str(&format!("path = {}\n", toml_string(&profile.path)));
    }
    out
}

/// A TOML basic string: `"` and `\` escaped, controls as `\uXXXX` — the
/// shapes here are names and config strings, but the escaper trusts nothing.
fn toml_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' | '\\' => out.push_str(&format!("\\{}", c)),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), HarnessError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|error| io_error("set mode on", path, error))
}

/// Writes a run config file private to the owner: 0600 at creation and
/// re-set after — a loose mode never survives a rewrite. Never a default:
/// DuckDB was found writing 0644 under a 0700 directory.
fn write_private(path: &Path, contents: &str) -> Result<(), HarnessError> {
    #[cfg(unix)]
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(path)
        .map_err(|error| io_error("write run config file", path, error))?;
    #[cfg(unix)]
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| io_error("set mode on", path, error))?;
    file.write_all(contents.as_bytes())
        .map_err(|error| io_error("write run config bytes", path, error))?;
    Ok(())
}
