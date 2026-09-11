//! The runner's OS sandbox: the fail-closed seam between a run's approved
//! runner scope and a child process actually being spawnable on this host.
//!
//! The policy type is [`RunSandbox`] — the filesystem roots a runner child
//! may read and write, and the `(host, port)` egress endpoints it may reach.
//! [`RunSandbox::prepare`] is the only entry point to a spawn configuration,
//! and it refuses everything the host cannot prove:
//!
//! - **Construction refuses what the platform cannot express.** On macOS the
//!   Seatbelt host position accepts only `*` or `localhost`, so any other
//!   `net_allow` host is refused at construction — never silently narrowed.
//!   On macOS the *host* allowlist is therefore enforced in-process by the
//!   M3-1 fetch policy, never by this sandbox, and the generated profile says
//!   so. On Linux every non-empty `net_allow` is refused (the netns denies
//!   all egress; Landlock's port rules have no host dimension) [UNVERIFIED].
//! - **The startup probe decides, never an assumption.** `prepare` runs a
//!   canary battery through the real generator output on this host — denied
//!   read, denied write, denied egress with `Operation not permitted`
//!   evidence, an allowed endpoint that actually lands on an accepting
//!   listener — and hands out [`SandboxProvision::Proven`] only when every
//!   required check passed ([`ProbeReport::proves_runner`]). There is no
//!   registered-with-a-warning and no degraded mode: a host that cannot be
//!   proven yields [`SandboxProvision::Refused`].
//! - **The registration consequence is structural.** [`RunnerSpawn`] is the
//!   only handle a runner tool can spawn through, and
//!   [`SandboxProvision::plan_capabilities`] strips the runner scope from the
//!   capabilities plan validation sees when nothing was proven — so a plan
//!   requesting the runner scope is refused exactly like any other
//!   unapproved capability. Windows fails closed by construction (U9): the
//!   probe records the absence, the spawn configuration does not exist.
//!
//! This module owns the confinement; the `run_program` tool that consumes
//! [`RunnerSpawn`] is M5-4 and is deliberately not wired here.

mod report;
mod validate;

pub use report::ProbeReport;
use spawn::SpawnPlatform;
pub use spawn::{RunnerSpawn, SandboxProvision};

use std::{
    io,
    path::{Path, PathBuf},
};

#[cfg(target_os = "linux")]
use std::sync::Arc;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
mod linux_canary;
#[cfg(target_os = "linux")]
mod linux_fork;
#[cfg(target_os = "linux")]
mod linux_sys;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "macos")]
mod macos_canary;
mod probe;
#[cfg(target_os = "linux")]
mod probe_linux;
#[cfg(target_os = "macos")]
mod probe_macos;
#[cfg(not(windows))]
mod probe_support;
mod spawn;

/// The runner child's sandbox policy: where it may read and write
/// (`fs_roots`), and the outbound egress it may reach (`net_allow`, as
/// `(host, port)` pairs).
///
/// Construction is the fail-closed gate for what the OS sandbox can express:
/// every root is canonicalised (Seatbelt matches the resolved path — spike
/// §4.2) and checked against the profile-injection text class; every
/// `net_allow` host must be expressible on this platform or the policy is
/// refused. On macOS the host component of a `net_allow` entry is **not**
/// enforced by the sandbox — Seatbelt cannot name a remote host — so the
/// host allowlist must be enforced in-process (M3-1 fetch policy) or the
/// entry must not exist. Nothing here narrows silently.
#[derive(Debug, Clone)]
pub struct RunSandbox {
    fs_roots: Vec<PathBuf>,
    net_allow: Vec<(String, u16)>,
}

impl RunSandbox {
    /// Builds the policy, refusing every shape the platform's sandbox cannot
    /// express. Roots are canonicalised before anything is stored (measured
    /// §4.2); `net_allow` hosts are validated per platform at construction.
    pub fn new(
        fs_roots: impl IntoIterator<Item = PathBuf>,
        net_allow: impl IntoIterator<Item = (String, u16)>,
    ) -> Result<Self, SandboxError> {
        let roots: Vec<PathBuf> = fs_roots
            .into_iter()
            .map(|root| validate::canonical_root(&root))
            .collect::<Result<_, _>>()?;
        if roots.is_empty() {
            return Err(SandboxError::NoRoots);
        }
        let mut net = Vec::new();
        for (host, port) in net_allow {
            validate::net_entry(&host, port)?;
            net.push((host, port));
        }
        Ok(Self {
            fs_roots: roots,
            net_allow: net,
        })
    }

    /// The canonicalised roots, in the order given.
    pub fn fs_roots(&self) -> &[PathBuf] {
        &self.fs_roots
    }

    /// The egress endpoints, as declared and platform-validated.
    pub fn net_allow(&self) -> &[(String, u16)] {
        &self.net_allow
    }

    /// The entry point: probes this host with this policy, and only a
    /// proven probe yields the runner's spawn configuration.
    ///
    /// `program_dir` is the directory holding the one allowlisted runner
    /// program (M5-4's parameter); it is canonicalised and profile-checked
    /// like any root before it is allowlisted for exec. The probe measures
    /// this host at this moment — a hardening environment that passed a
    /// previous run's probe does not carry forward (spike §9.3).
    pub fn prepare(&self, program_dir: &Path) -> Result<SandboxProvision, SandboxError> {
        let program_dir = validate::canonical_root(program_dir)?;
        let report = probe::run(self);
        if !report.proves_runner() {
            return Ok(SandboxProvision::Refused(report));
        }
        Ok(SandboxProvision::Proven {
            spawn: RunnerSpawn::new(
                self.fs_roots.clone(),
                self.net_allow.clone(),
                program_dir.clone(),
                self.spawn_platform(&program_dir)?,
            ),
            report,
        })
    }

    fn spawn_platform(&self, program_dir: &Path) -> Result<SpawnPlatform, SandboxError> {
        #[cfg(target_os = "macos")]
        let platform = SpawnPlatform::Seatbelt {
            profile: macos::seatbelt_profile(self, &[program_dir.to_path_buf()])?,
        };
        #[cfg(target_os = "linux")]
        let platform = SpawnPlatform::Landlock(Arc::new(linux::Confinement::new(
            self,
            &[program_dir.to_path_buf()],
        )?));
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let platform = SpawnPlatform::Absent;
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let _ = program_dir;
        Ok(platform)
    }
}

/// Why a policy could not be prepared. Every variant is terminal: nothing
/// here degrades to a weaker sandbox.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SandboxError {
    /// A root or the program directory could not be canonicalised.
    #[error("sandbox root cannot be resolved: {context}")]
    Io {
        context: String,
        #[source]
        source: io::Error,
    },

    /// A root is outside the conservative profile-safe text class, is not a
    /// directory, or is the filesystem root itself. Refused, never escaped:
    /// the escaping semantics are unmeasured.
    #[error("sandbox root is refused for the profile language ({reason}): {path}")]
    RootNotSafeForProfile { path: String, reason: String },

    /// A `net_allow` host is not expressible by this platform's sandbox.
    /// Enforcing it in-process (fetch policy) or refusing the capability is
    /// the caller's decision; the sandbox never narrows the host silently.
    #[error("net_allow host `{host}` is not expressible on {platform}: {reason}")]
    NetHostNotExpressible {
        host: String,
        platform: &'static str,
        reason: &'static str,
    },

    /// Port 0 is not an enforceable egress declaration.
    #[error("net_allow port {port} is not an enforceable endpoint")]
    NetPortInvalid { port: u16 },

    /// The policy declares no filesystem root; a runner child with nowhere
    /// to write is refused rather than guessed at.
    #[error("RunSandbox requires at least one filesystem root")]
    NoRoots,

    /// No allowlisted program directory was given for the profile.
    #[error("no runner program directory was allowlisted")]
    NoProgramDirs,

    /// The generated profile still carries a placeholder. A quoted leftover
    /// is silently accepted and matches nothing (measured), so this fails
    /// closed at generation instead of becoming a permissive half-profile.
    #[error("generated profile still carries an unsubstituted placeholder; refused")]
    PlaceholderLeft,
}
