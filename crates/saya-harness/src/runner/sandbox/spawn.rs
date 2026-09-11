//! The spawn configuration handed out by a proven sandbox: what the runner
//! tool (M5-4) consumes to confine a child, and the provision type that
//! carries the fail-closed verdict.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(target_os = "linux")]
use std::{os::unix::process::CommandExt, sync::Arc};

use saya_types::Capabilities;

use super::ProbeReport;

#[cfg(target_os = "linux")]
use super::linux;
#[cfg(target_os = "macos")]
use super::macos;

/// The per-platform spawn mechanics behind a proven [`RunnerSpawn`].
///
/// `Clone` because the configuration is immutable and a run may hand it to
/// more than one tool instance; cloning it grants nothing — the proven arm
/// of `prepare` remains the only source.
#[derive(Debug, Clone)]
pub(super) enum SpawnPlatform {
    /// The generated Seatbelt profile (macOS, measured; see `macos`).
    #[cfg(target_os = "macos")]
    Seatbelt { profile: String },
    /// The Landlock + namespace confinement (Linux, UNVERIFIED on any Linux
    /// host; only reachable where the probe proved it).
    #[cfg(target_os = "linux")]
    Landlock(Arc<linux::Confinement>),
    /// Never constructed: no platform outside macOS and Linux has a sandbox
    /// primitive this design targets (U9), and no probe proves one.
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    Absent,
}

/// The spawn configuration the runner tool (M5-4) needs, handed out only
/// where the startup probe proved the sandbox on this host. There is no way
/// to construct one otherwise: [`SandboxProvision::Proven`] is the only
/// source. `Clone` because the configuration is immutable — cloning grants
/// nothing the proven arm did not hand out.
#[derive(Debug, Clone)]
pub struct RunnerSpawn {
    fs_roots: Vec<PathBuf>,
    net_allow: Vec<(String, u16)>,
    program_dir: PathBuf,
    platform: SpawnPlatform,
}

impl RunnerSpawn {
    /// Constructible only inside the sandbox module: the proven arm of
    /// `prepare` is the only source.
    pub(super) fn new(
        fs_roots: Vec<PathBuf>,
        net_allow: Vec<(String, u16)>,
        program_dir: PathBuf,
        platform: SpawnPlatform,
    ) -> Self {
        Self {
            fs_roots,
            net_allow,
            program_dir,
            platform,
        }
    }

    /// A command to exec `program` under this confinement — on macOS the
    /// `sandbox-exec -p <profile> <program> ...` prefix, on Linux the child
    /// binary with the confinement installed pre-exec. The child's cwd is
    /// pinned to the first root (measured: a cwd outside the roots is
    /// refused at startup and noisy — spike §4.2); M5-4 owns the final argv,
    /// environment, and cwd choice on top of this.
    pub fn command(&self, program: &Path) -> Command {
        #[cfg(target_os = "macos")]
        let mut command = match &self.platform {
            SpawnPlatform::Seatbelt { profile } => {
                let mut c = Command::new(macos::SANDBOX_EXEC);
                c.arg("-p").arg(profile).arg(program);
                c
            }
        };
        #[cfg(target_os = "linux")]
        let mut command = match &self.platform {
            SpawnPlatform::Landlock(confinement) => {
                let mut c = Command::new(program);
                let closure = linux::pre_exec_closure(Arc::clone(confinement));
                // Safety: the closure runs only inside the forked child
                // before exec, per `pre_exec`'s contract.
                unsafe {
                    c.pre_exec(closure);
                }
                c
            }
        };
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        // Never reached: no spawn configuration exists on platforms without a
        // sandbox primitive this design targets (U9) — RunnerSpawn is
        // constructible only in the proven arm of `prepare`.
        let mut command = Command::new(program);
        command.current_dir(&self.fs_roots[0]);
        command
    }

    /// The directory the one allowlisted runner program lives in (M5-4's
    /// parameter, canonicalised at prepare time).
    pub fn program_dir(&self) -> &Path {
        &self.program_dir
    }

    /// The canonicalised roots the child is confined to.
    pub fn fs_roots(&self) -> &[PathBuf] {
        &self.fs_roots
    }

    /// The egress endpoints the child may reach — the enforcement shape, not
    /// a re-statement of the policy: the host component is enforced
    /// in-process, never by this sandbox (see [`RunSandbox`]).
    pub fn net_allow(&self) -> &[(String, u16)] {
        &self.net_allow
    }

    /// The macOS Seatbelt profile text, when this spawn is Seatbelt-based.
    pub fn profile(&self) -> Option<&str> {
        #[cfg(target_os = "macos")]
        return match &self.platform {
            SpawnPlatform::Seatbelt { profile } => Some(profile),
        };
        #[cfg(target_os = "linux")]
        return match &self.platform {
            SpawnPlatform::Landlock(_) => None,
        };
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        return match &self.platform {
            SpawnPlatform::Absent => None,
        };
    }
}

/// What preparing the sandbox produced. The runner tool joins the
/// composition root's universe only in the `Proven` arm — a `Refused`
/// provision is the absence of the capability, exactly like a non-approved
/// scope, never a warning.
#[derive(Debug)]
pub enum SandboxProvision {
    /// The probe proved the sandbox on this host; here is the spawn
    /// configuration the runner may use, with the report that proved it.
    Proven {
        spawn: RunnerSpawn,
        report: ProbeReport,
    },
    /// Not proven on this host: no spawn configuration exists, the runner
    /// tool is not registered, and runner scopes are refused at plan time.
    Refused(ProbeReport),
}

impl SandboxProvision {
    /// The spawn configuration, if one exists.
    pub fn spawn(&self) -> Option<&RunnerSpawn> {
        match self {
            Self::Proven { spawn, .. } => Some(spawn),
            Self::Refused(_) => None,
        }
    }

    /// The probe report this provision registered with — the evidence, in
    /// both arms.
    pub fn report(&self) -> &ProbeReport {
        match self {
            Self::Proven { report, .. } => report,
            Self::Refused(report) => report,
        }
    }

    /// The approved capabilities as plan validation must see them: the
    /// runner scope survives only where the sandbox was proven. A plan step
    /// requesting the runner scope against the refused form is rejected with
    /// the ordinary needs-approval outcome — the capability is absent, not
    /// degraded.
    pub fn plan_capabilities(&self, approved: &Capabilities) -> Capabilities {
        let mut caps = approved.clone();
        if matches!(self, Self::Refused(_)) {
            caps.runner = None;
        }
        caps
    }
}
