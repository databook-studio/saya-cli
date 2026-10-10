//! Confinement of a child command to exactly one verified managed checkout.
//!
//! [`confine_checkout`] proves a sandbox whose single file-system root is the
//! owned checkout itself. The confinement therefore denies writes to the
//! common Git metadata, to sibling checkouts and to the original workspace,
//! because they all lie outside that single root. The checkout's own `.git`
//! pointer file is inside the root and writable; this is accepted because no
//! other path becomes writable, and a child `git` cannot resolve the pointer
//! to the common metadata by design.
//!
//! A [`VerifiedCheckout`] holds only at the moment of verification. Callers
//! must call `verify_owned_checkout` and [`confine_checkout`] in one flow and
//! never cache a [`ConfinedCheckout`] as standing authority.
//!
//! Every child command spawned through [`ConfinedCheckout::command`] sees
//! only the child environment's six variables (`HOME`, `LANG`, `LC_ALL`,
//! `PATH`, `TMPDIR`, `XDG_CONFIG_HOME`); the harness process's environment is
//! never inherited, so no provider, database, cloud, SSH or proxy secret can
//! reach the child. The confinement hands out capability only in
//! already-confined form: [`ConfinedCheckout::platform_support`] states the
//! trusted host support the proof establishes, and
//! [`ConfinedCheckout::runner_tool`] gives the child run_program tool with
//! that same environment pinned at spawn.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use saya_types::{InterpreterScope, ManagedCheckoutId, RunnerScope};

use crate::coding::policy::{CommandCheckoutProfile, PlatformSupport};
use crate::coding::worktree::{ChildEnvironment, ChildEnvironmentError, VerifiedCheckout};
use crate::runner::sandbox::{RunSandbox, RunnerSpawn, SandboxError, SandboxProvision};
use crate::runner::{RunProgram, SharedCredentialSource};

/// A spawn configuration proven for exactly one verified managed checkout.
///
/// The held [`RunnerSpawn`] grants exactly one file-system root — the owned
/// checkout — and no network. [`Debug`] reports the checkout id only; it
/// never reports a path.
pub struct ConfinedCheckout {
    spawn: RunnerSpawn,
    environment: ChildEnvironment,
    checkout_id: ManagedCheckoutId,
}

impl fmt::Debug for ConfinedCheckout {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConfinedCheckout")
            .field("checkout_id", &self.checkout_id)
            .finish()
    }
}

impl ConfinedCheckout {
    /// The managed checkout the confinement was proven for.
    pub fn checkout_id(&self) -> &ManagedCheckoutId {
        &self.checkout_id
    }

    /// A command to exec `program` under the confinement; cwd is the owned
    /// checkout (first and only root). The child's environment is built
    /// here, not inherited: every inherited variable is cleared and exactly
    /// the six allowlisted variables are set. This is the only command path
    /// the confinement offers.
    pub fn command(&self, program: &Path) -> std::process::Command {
        let mut command = self.spawn.command(program);
        self.environment.apply(&mut command);
        command
    }

    /// The trusted host support this proven confinement establishes:
    /// commands under an isolated private checkout, edits confined to
    /// exactly that checkout, no network, no database reads. The presence of
    /// a [`ConfinedCheckout`] is itself the proof — `confine_checkout`
    /// refuses everything not proven (single root, empty network) — so this
    /// only states what the evidence already establishes.
    pub fn platform_support(&self) -> PlatformSupport {
        PlatformSupport {
            workspace_edits: true,
            command_profile: Some(CommandCheckoutProfile::IsolatedPrivateCheckout),
            network_fetch: false,
            database_reads: false,
        }
    }

    /// The child run_program tool over this confinement: the proven spawn,
    /// the caller-narrowed scopes, and the confinement's child environment
    /// handed out in already-confined form. The environment's variables are
    /// pinned after the spawn's `env_clear` and before credentials, so the
    /// child's whole environment is exactly those six variables and its
    /// credentials stay empty — `with_credentials` is never called here.
    ///
    /// `record_dir` MUST be a trusted directory outside the confined
    /// checkout (the composition root's private run-record directory): the
    /// host writes outcome records there, so a directory inside the checkout
    /// would let a child redirect host writes with a symlink and would show
    /// up as an unassigned change at receipt time; it is never defaulted
    /// into the checkout.
    pub fn runner_tool(
        &self,
        runner: Option<RunnerScope>,
        interpreters: Option<InterpreterScope>,
        default_timeout: Duration,
        resolver: SharedCredentialSource,
        record_dir: PathBuf,
    ) -> RunProgram {
        RunProgram::for_step(
            self.spawn.clone(),
            runner,
            interpreters,
            default_timeout,
            resolver,
        )
        .with_pinned_env(self.environment.variables().to_vec())
        .with_record_dir(record_dir)
    }
}

/// Failure modes of [`confine_checkout`].
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum ConfinementError {
    /// The sandbox could not be built or provisioned for the checkout.
    #[error("the sandbox could not be built for the checkout")]
    Sandbox(#[from] SandboxError),
    /// The host refused to prove command confinement; there is no degraded
    /// mode and no spawn configuration.
    #[error("command confinement is not proven on this host")]
    Unsupported,
    /// The proven confinement does not match the checkout: the file-system
    /// roots are not exactly the single checkout root or the network is not
    /// fully denied.
    #[error("the proven confinement does not match the checkout")]
    RootMismatch,
    /// The child environment could not be prepared for the confinement.
    #[error("the child environment could not be prepared")]
    Environment(#[from] ChildEnvironmentError),
}

/// Confine a child command to exactly one verified managed checkout.
///
/// Builds a [`RunSandbox`] whose roots are exactly the checkout and whose
/// network allowlist is empty, then provisions it for `program_dir`;
/// `with_process_fork` is never used. A host refusal yields
/// [`ConfinementError::Unsupported`] — no degraded mode, no spawn
/// configuration. A proven configuration that does not keep exactly one
/// file-system root and an empty network allowlist yields
/// [`ConfinementError::RootMismatch`].
///
/// Pair with `verify_owned_checkout` in one flow; see the module docs.
///
/// The child environment is prepared only after the sandbox is proven and
/// checked, so a missing or unusable program directory still fails first as
/// [`ConfinementError::Sandbox`]; an unusable private directory fails as
/// [`ConfinementError::Environment`].
pub fn confine_checkout(
    verified: &VerifiedCheckout,
    program_dir: &Path,
) -> Result<ConfinedCheckout, ConfinementError> {
    let sandbox = RunSandbox::new(
        [PathBuf::from(verified.path().as_str())],
        std::iter::empty(),
    )?;
    let spawn = match sandbox.prepare(program_dir)? {
        SandboxProvision::Proven { spawn, .. } => spawn,
        SandboxProvision::Refused(_) => return Err(ConfinementError::Unsupported),
    };
    if spawn.fs_roots().len() != 1 || !spawn.net_allow().is_empty() {
        return Err(ConfinementError::RootMismatch);
    }
    let environment = ChildEnvironment::prepare(Path::new(verified.path().as_str()), program_dir)?;
    Ok(ConfinedCheckout {
        spawn,
        environment,
        checkout_id: verified.checkout_id().clone(),
    })
}
