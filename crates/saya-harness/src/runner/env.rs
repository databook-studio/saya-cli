//! The runner child's environment: empty by default, credentials only under
//! all four conditions. This module owns the credential shape and the
//! resolution seam; the injection itself happens in `spawn` on the command
//! that was already validated, sandboxed, and argv-typed.
//!
//! The four conditions, and where each one lives:
//!
//! 1. **Declared in the approved plan** — a run-time gate. The composition
//!    root builds the step's credential list from the approved plan alone;
//!    a credential not on that list is never constructed into the tool. The
//!    escape battery removes this condition and watches the variable vanish
//!    from the child's actual environment.
//! 2. **Sandboxed** — structural. A credential list can only be attached to
//!    a tool that was constructed around a proven [`RunnerSpawn`], and no
//!    `RunnerSpawn` exists where the startup probe did not prove the
//!    sandbox. There is no unsandboxed path that could carry a credential.
//! 3. **References-only config** — structural. A [`Credential`] carries a
//!    [`SecretRef`]; there is no constructor from a literal value, so an
//!    inline secret cannot exist at this seam. Values enter only through
//!    [`CredentialSource::resolve`] at call time.
//! 4. **redact() applied to all captured output** — structural. The output
//!    path redacts unconditionally (`output::capture`); no call site can
//!    skip it.
//!
//! Any one missing means no credential — and three of the four cannot be
//! violated by a caller; only "declared" is a per-step decision.

use std::collections::BTreeMap;
use std::sync::Arc;

use saya_types::{SecretRef, is_bare_name};

use super::RunnerError;

/// One credential a step declared: the run-scoped name it is known by, the
/// environment variable it is injected under, and the reference its value is
/// resolved from. There is no constructor from a value — references-only
/// config is a property of the type, so an inline secret cannot exist on
/// this path.
pub struct Credential {
    name: String,
    env_var: String,
    reference: SecretRef,
}

impl Credential {
    /// Builds a declared credential. `name` must be a bare run-scoped name;
    /// `env_var` must be a well-formed environment variable name (ASCII
    /// letters, digits, underscores, not starting with a digit) — a variable
    /// the child's `NAME=VALUE` environment cannot represent is refused.
    pub fn new(
        name: impl Into<String>,
        env_var: impl Into<String>,
        reference: SecretRef,
    ) -> Result<Self, RunnerError> {
        let name = name.into();
        let env_var = env_var.into();
        if !is_bare_name(&name) {
            return Err(RunnerError::CredentialInvalid {
                credential: name,
                reason: "the credential name must be a bare run-scoped name",
            });
        }
        let well_formed = !env_var.is_empty()
            && env_var.len() <= 64
            && !env_var.chars().next().is_some_and(|c| c.is_ascii_digit())
            && env_var
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !well_formed {
            return Err(RunnerError::CredentialInvalid {
                credential: env_var,
                reason: "the environment variable name must be ASCII letters, digits and \
                         underscores, and must not start with a digit",
            });
        }
        Ok(Self {
            name,
            env_var,
            reference,
        })
    }

    /// The run-scoped name the credential was declared under.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The environment variable the credential is injected under.
    pub fn env_var(&self) -> &str {
        &self.env_var
    }

    /// Resolves the declared reference through `source`. A reference that
    /// cannot be resolved fails the call — a declared credential that
    /// silently did not inject would leave the child to fail in its place.
    pub(crate) fn resolved(&self, source: &dyn CredentialSource) -> Result<String, RunnerError> {
        source
            .resolve(&self.reference)
            .map_err(|detail| RunnerError::CredentialUnresolved {
                credential: self.name.clone(),
                detail,
            })
    }
}

/// The seam a resolved credential value crosses — the only one. A value is
/// resolved from a reference at call time and handed straight to the child's
/// environment; it is never stored, logged, or rendered. The error detail is
/// text the adapter chose — it must name the failure, never the value.
pub trait CredentialSource: Send + Sync {
    fn resolve(&self, reference: &SecretRef) -> Result<String, String>;
}

/// A reference [`CredentialSource`] over a static map: the test seam, and
/// the reference adapter shape a composition root wraps around the config
/// layer's own resolver.
pub struct StaticCredentialSource {
    values: BTreeMap<String, String>,
}

impl StaticCredentialSource {
    pub fn new(values: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            values: values.into_iter().collect(),
        }
    }
}

impl CredentialSource for StaticCredentialSource {
    fn resolve(&self, reference: &SecretRef) -> Result<String, String> {
        match reference {
            SecretRef::Env { env } => self
                .values
                .get(env)
                .cloned()
                .ok_or_else(|| format!("environment variable {env} is not set")),
            SecretRef::File { file } => std::fs::read_to_string(file)
                .map(|value| value.trim_end().to_owned())
                .map_err(|_| "secret file could not be read".to_owned()),
            SecretRef::Keyring { .. } => {
                Err("keyring secret references are unavailable in this runtime".to_owned())
            }
        }
    }
}

/// Injects every declared credential's resolved value into `command`'s
/// environment — the child's environment is otherwise empty, so what is
/// declared here is the whole of what the child can see.
pub(crate) fn inject(
    command: &mut std::process::Command,
    credentials: &[Credential],
    source: &dyn CredentialSource,
) -> Result<(), RunnerError> {
    for credential in credentials {
        command.env(credential.env_var(), credential.resolved(source)?);
    }
    Ok(())
}

/// The shared resolver handle a tool carries.
pub type SharedCredentialSource = Arc<dyn CredentialSource + Send + Sync>;
