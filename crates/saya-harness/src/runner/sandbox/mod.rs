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
//!   fetch policy, never by this sandbox, and the generated profile says
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
//!   only handle a confined tool can spawn through, and a host that cannot be
//!   proven yields [`SandboxProvision::Refused`] — no spawn configuration
//!   exists, so the confined tool never joins the universe. Windows fails
//!   closed by construction: the
//!   probe records the absence, the spawn configuration does not exist.
//!
//! This module owns the confinement; the shell is the tool that consumes
//! [`RunnerSpawn`].

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
/// every root is canonicalised (Seatbelt matches the resolved path) and
/// checked against the profile-injection text class; every
/// `net_allow` host must be expressible on this platform or the policy is
/// refused. On macOS the host component of a `net_allow` entry is **not**
/// enforced by the sandbox — Seatbelt cannot name a remote host — so the
/// host allowlist must be enforced in-process by the fetch policy or the
/// entry must not exist. Nothing here narrows silently.
#[derive(Debug, Clone)]
pub struct RunSandbox {
    fs_roots: Vec<PathBuf>,
    net_allow: Vec<(String, u16)>,
    /// The measured reason `(allow process-fork)` was granted, when it was.
    /// Deny by default means a child that forks is refused with
    /// `fork: Operation not permitted` (measured), so any
    /// allowlisted program that forks needs this opt-in — and the reason
    /// with which it was granted is the measured evidence, carried verbatim
    /// into the generated profile as the comment a reviewer reads.
    process_fork: Option<String>,
    /// Paths denied writes inside an allowing root, resolved at construction.
    write_denied: Vec<PathBuf>,
    /// Roots a child may read and execute but never write: the host's
    /// toolchain directories, so a sandboxed `cargo test` can reach the
    /// compiler without widening the checkout. Canonicalised at construction.
    read_only_roots: Vec<PathBuf>,
}

impl RunSandbox {
    /// Builds the policy, refusing every shape the platform's sandbox cannot
    /// express. Roots are canonicalised before anything is stored;
    /// `net_allow` hosts are validated per platform at construction.
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
            process_fork: None,
            write_denied: Vec::new(),
            read_only_roots: Vec::new(),
        })
    }

    /// Adds roots a child may read and execute but never write. The shell
    /// uses this for the host's toolchain directories: a sandboxed
    /// `cargo test` needs the compiler and its libraries, and none of that
    /// may become writable. Every root is canonicalised and profile-checked
    /// like an `fs_roots` entry; a root that cannot be resolved refuses the
    /// policy rather than degrading.
    pub fn with_read_only_roots(
        mut self,
        roots: impl IntoIterator<Item = PathBuf>,
    ) -> Result<Self, SandboxError> {
        for root in roots {
            let resolved = validate::canonical_root(&root)?;
            if !self.read_only_roots.contains(&resolved) {
                self.read_only_roots.push(resolved);
            }
        }
        Ok(self)
    }

    /// The canonicalised read-only roots, in the order given.
    pub fn read_only_roots(&self) -> &[PathBuf] {
        &self.read_only_roots
    }

    /// Deny writes to `path` and everything under it, even inside a write
    /// root; `path` must be inside one of `fs_roots` and need not exist yet.
    /// Its parent is canonicalised and the last component re-joined.
    pub fn with_write_denied(mut self, path: impl Into<PathBuf>) -> Result<Self, SandboxError> {
        let resolved = validate::write_denial(&path.into(), &self.fs_roots)?;
        self.write_denied.push(resolved);
        Ok(self)
    }

    /// The write-denied paths, resolved against a canonical root, in order.
    pub fn write_denied(&self) -> &[PathBuf] {
        &self.write_denied
    }

    /// Grants `(allow process-fork)` to the generated profile, with the
    /// measured reason it is granted for. The reason becomes the
    /// profile's own comment — it must carry the same conservative text
    /// class every other profile substitution carries, so a reason shaped
    /// like profile language is refused here. On Linux this is a deliberate
    /// no-op: nothing in the Landlock + namespace confinement restricts
    /// fork, so there is nothing to grant [UNVERIFIED platform].
    pub fn with_process_fork(mut self, reason: impl Into<String>) -> Result<Self, SandboxError> {
        let reason = reason.into();
        if !validate::text_safe(&reason) {
            return Err(SandboxError::RootNotSafeForProfile {
                path: reason,
                reason: "the process-fork reason is substituted into a profile comment and \
                         must carry the safe profile text class"
                    .into(),
            });
        }
        self.process_fork = Some(reason);
        Ok(self)
    }

    /// The measured reason `process-fork` was granted, when it was.
    pub fn process_fork_reason(&self) -> Option<&str> {
        self.process_fork.as_deref()
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
    /// program; it is canonicalised and profile-checked
    /// like any root before it is allowlisted for exec. The probe measures
    /// this host at this moment — a hardening environment that passed a
    /// previous run's probe does not carry forward.
    pub fn prepare(&self, program_dir: &Path) -> Result<SandboxProvision, SandboxError> {
        self.prepare_with_program_dirs(std::slice::from_ref(&program_dir.to_path_buf()))
    }

    /// As [`RunSandbox::prepare`], for a child that execs from more than one
    /// directory. The workspace shell is the caller: `/bin/sh` lives in
    /// `/bin` while the programs its `PATH` names live in `/usr/bin`,
    /// `/usr/sbin` and `/sbin`, and no single directory holds both. Every
    /// directory is canonicalised and profile-checked exactly like
    /// `prepare`'s one, and the probe still measures the profile the runner
    /// would actually generate — a host that cannot be proven refuses here
    /// too. The first directory is the spawn's `program_dir`, the one
    /// `run_program` resolves names in.
    pub fn prepare_with_program_dirs(
        &self,
        program_dirs: &[PathBuf],
    ) -> Result<SandboxProvision, SandboxError> {
        if program_dirs.is_empty() {
            return Err(SandboxError::NoProgramDirs);
        }
        let dirs: Vec<PathBuf> = program_dirs
            .iter()
            .map(|dir| validate::canonical_root(dir))
            .collect::<Result<_, _>>()?;
        let report = probe::run(self);
        if !report.proves_runner() {
            return Ok(SandboxProvision::Refused(report));
        }
        Ok(SandboxProvision::Proven {
            spawn: RunnerSpawn::new(
                self.fs_roots.clone(),
                self.net_allow.clone(),
                dirs[0].clone(),
                self.spawn_platform(&dirs)?,
            ),
            report,
        })
    }

    fn spawn_platform(&self, program_dirs: &[PathBuf]) -> Result<SpawnPlatform, SandboxError> {
        #[cfg(target_os = "macos")]
        let platform = SpawnPlatform::Seatbelt {
            profile: macos::seatbelt_profile(self, program_dirs)?,
        };
        #[cfg(target_os = "linux")]
        let platform =
            SpawnPlatform::Landlock(Arc::new(linux::Confinement::new(self, program_dirs)?));
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let platform = SpawnPlatform::Absent;
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let _ = program_dirs;
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

    /// A write-denial path is not inside any filesystem root: a policy bug,
    /// refused at construction rather than emitted as a rule matching nothing.
    #[error("write denial `{path}` is not inside any sandbox filesystem root")]
    WriteDenialOutsideRoots { path: String },

    /// This platform's sandbox cannot carve a denied path out of an allowed
    /// tree (Landlock is allow-only); refused at prepare time, fail closed.
    #[error("a write denial is not expressible on {platform}: {reason}")]
    WriteDenialNotExpressible {
        platform: &'static str,
        reason: &'static str,
    },

    /// The generated profile still carries a placeholder. A quoted leftover
    /// is silently accepted and matches nothing (measured), so this fails
    /// closed at generation instead of becoming a permissive half-profile.
    #[error("generated profile still carries an unsubstituted placeholder; refused")]
    PlaceholderLeft,
}
