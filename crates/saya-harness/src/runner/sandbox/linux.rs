//! The Linux confinement builder: Landlock filesystem rules plus an
//! unprivileged user+network namespace, written to the draft plan
//! (`docs/sandbox-profile-linux.md.draft`), over the raw syscall layer in
//! `linux_sys` — no `landlock` dependency (see the dependency note below).
//!
//! **UNVERIFIED ON ANY LINUX HOST.** The M5-3 spike ran on macOS 26.6.2 and
//! observed no Linux behaviour; every factual claim marked [UNVERIFIED]
//! cites the kernel's own documentation (landlock(7), user_namespaces(7),
//! unshare(2)), never observation. This file is written so the startup
//! probe — not these assumptions — decides whether it may be used: on a
//! Linux host the probe runs the enforcement canaries through exactly this
//! code, and any unmeasured assumption that is wrong fails a required check,
//! which fails the verdict, which leaves the runner unregistered. Nothing
//! here can enable the runner by itself.
//!
//! Deliberate shape choices, each a reviewer decision point the draft left
//! open (draft §4):
//! - `net_allow` must be empty on Linux — a fresh netns has no network at
//!   all, so no `net_allow` entry can be honoured inside it [UNVERIFIED], and
//!   Landlock's port rules have no host dimension. Enforcing egress
//!   selectively needs a proxy component this design does not have; the
//!   runner therefore fails closed on hosts where the policy cannot be
//!   honoured — including LTS kernels where unprivileged namespaces are
//!   commonly disabled [UNVERIFIED].
//! - The handled set is every filesystem right the measured ABI supports,
//!   and this design refuses ABI < 3 (TRUNCATE): handling fewer rights than
//!   the kernel supports silently unrestricts (draft §2.1).
//! - Order: no-new-privs → Landlock restrict → unshare user → uid/gid maps
//!   → unshare net [UNVERIFIED order; landlock-before-unshare chosen so the
//!   ruleset does not depend on the new user namespace].
//! - The `landlock` crate is deliberately not a dependency: it is new and
//!   its `cargo audit` gate should run when a Linux host has measured this
//!   design, not before (spike §7). If it is ever adopted, this file's
//!   marshalling is what it replaces.

use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use super::linux_sys as sys;
use super::{RunSandbox, SandboxError};

/// The ABI this design requires: TRUNCATE (ABI 3, Linux 6.2 [UNVERIFIED]) is
/// part of the handled set, so a kernel below it is refused rather than
/// silently under-restricting.
pub(super) const MIN_LANDLOCK_ABI: u32 = 3;

/// The read/write universe of an `fs_roots` root [UNVERIFIED access set —
/// draft §2.3: everything except EXECUTE and REFER].
const ROOT_ALLOWED: u64 = sys::LANDLOCK_ACCESS_FS_WRITE_FILE
    | sys::LANDLOCK_ACCESS_FS_READ_FILE
    | sys::LANDLOCK_ACCESS_FS_READ_DIR
    | sys::LANDLOCK_ACCESS_FS_REMOVE_DIR
    | sys::LANDLOCK_ACCESS_FS_REMOVE_FILE
    | sys::LANDLOCK_ACCESS_FS_MAKE_CHAR
    | sys::LANDLOCK_ACCESS_FS_MAKE_DIR
    | sys::LANDLOCK_ACCESS_FS_MAKE_REG
    | sys::LANDLOCK_ACCESS_FS_MAKE_SOCK
    | sys::LANDLOCK_ACCESS_FS_MAKE_FIFO
    | sys::LANDLOCK_ACCESS_FS_MAKE_BLOCK
    | sys::LANDLOCK_ACCESS_FS_MAKE_SYM
    | sys::LANDLOCK_ACCESS_FS_TRUNCATE;

/// The exec universe of the allowlisted program directories and the loader
/// paths [UNVERIFIED path set — unmeasured on any Linux host; the probe's
/// exec canary decides].
const EXEC_ALLOWED: u64 = sys::LANDLOCK_ACCESS_FS_EXECUTE
    | sys::LANDLOCK_ACCESS_FS_READ_FILE
    | sys::LANDLOCK_ACCESS_FS_READ_DIR;

/// The loader directories a dynamically-linked child needs [UNVERIFIED; a
/// path that cannot be allowed fails closed at construction rather than
/// leaving the child unable to start with the cause hidden].
const LOADER_DIRS: &[&str] = &["/lib", "/lib64", "/usr/lib"];

/// Everything the pre-exec confinement applies: the Landlock ruleset (every
/// fs right the measured ABI supports, one PATH_BENEATH rule per root and
/// program directory), then the user+network namespace. Constructed once,
/// applied in the forked child — after fork the process is single-threaded
/// by fork(2)'s contract, which is what unshare(CLONE_NEWUSER) requires
/// [UNVERIFIED per unshare(2)].
#[derive(Debug)]
pub(super) struct Confinement {
    handled_access_fs: u64,
    /// One (parent path, allowed access) per PATH_BENEATH rule, pre-marshalled
    /// so `apply` performs no allocation after the fork.
    rules: Vec<(std::ffi::CString, u64)>,
    uid_map_content: std::ffi::CString,
    gid_map_content: std::ffi::CString,
    proc_setgroups: std::ffi::CString,
    proc_uid_map: std::ffi::CString,
    proc_gid_map: std::ffi::CString,
}

impl Confinement {
    /// Builds the confinement the draft describes, refusing the shapes this
    /// design cannot honour (ABI too old, paths that cannot be marshalled).
    pub(super) fn new(sb: &RunSandbox, program_dirs: &[PathBuf]) -> Result<Self, SandboxError> {
        let abi = sys::landlock_abi().map_err(|source| SandboxError::Io {
            context: "landlock ABI version query".into(),
            source,
        })?;
        if abi < MIN_LANDLOCK_ABI {
            return Err(SandboxError::Io {
                context: format!(
                    "Landlock ABI {abi} is below this design's minimum ({MIN_LANDLOCK_ABI}); \
                     refusing, never under-restricting"
                ),
                source: io::Error::new(io::ErrorKind::Unsupported, "landlock abi"),
            });
        }
        // handled = every fs right the ABI supports [UNVERIFIED ABI→rights
        // table; the enforcement canary proves whatever is claimed].
        let handled_access_fs = ROOT_ALLOWED | sys::LANDLOCK_ACCESS_FS_REFER;
        let mut rules = Vec::new();
        for path in sb
            .fs_roots()
            .iter()
            .map(|p| p.as_path())
            .chain(program_dirs.iter().map(|p| p.as_path()))
            .chain(LOADER_DIRS.iter().map(Path::new))
        {
            let allowed = if program_dirs.iter().any(|d| d.as_path() == path)
                || LOADER_DIRS.iter().any(|d| Path::new(d) == path)
            {
                EXEC_ALLOWED
            } else {
                ROOT_ALLOWED
            };
            rules.push((marshal(path)?, allowed));
        }
        Ok(Self {
            handled_access_fs,
            rules,
            uid_map_content: std::ffi::CString::new(format!("0 {} 1\n", sys::getuid()))
                .map_err(|_| marshal_error("the uid map content"))?,
            gid_map_content: std::ffi::CString::new(format!("0 {} 1\n", sys::getgid()))
                .map_err(|_| marshal_error("the gid map content"))?,
            proc_setgroups: marshal(Path::new("/proc/self/setgroups"))?,
            proc_uid_map: marshal(Path::new("/proc/self/uid_map"))?,
            proc_gid_map: marshal(Path::new("/proc/self/gid_map"))?,
        })
    }

    /// Applies the whole confinement to the calling (forked) child: no-new-
    /// privs, the Landlock ruleset, then the user+network namespace per the
    /// draft's order [UNVERIFIED — the probe measures each step's failure
    /// surface; nothing falls back].
    pub(super) fn apply(&self) -> io::Result<()> {
        sys::no_new_privs()?;
        let ruleset_fd = sys::create_ruleset(self.handled_access_fs)?;
        let restricted = self.restrict_under(ruleset_fd);
        unsafe { libc::close(ruleset_fd) };
        restricted?;
        sys::unshare_user()?;
        sys::write_proc(&self.proc_setgroups, "deny")?;
        sys::write_proc(
            &self.proc_uid_map,
            self.uid_map_content.to_str().unwrap_or_default(),
        )?;
        sys::write_proc(
            &self.proc_gid_map,
            self.gid_map_content.to_str().unwrap_or_default(),
        )?;
        sys::unshare_net()?;
        Ok(())
    }

    fn restrict_under(&self, ruleset_fd: libc::c_int) -> io::Result<()> {
        for (path, allowed) in &self.rules {
            let parent_fd = sys::open_path(path)?;
            let added = sys::add_path_beneath(ruleset_fd, parent_fd, *allowed);
            unsafe { libc::close(parent_fd) };
            added?;
        }
        sys::restrict_self(ruleset_fd)
    }
}

/// The forked-child confinement entry [`RunnerSpawn`](super::RunnerSpawn)
/// installs via `CommandExt::pre_exec`. The closure performs no allocation:
/// every path is pre-marshalled into the [`Confinement`].
pub(super) fn pre_exec_closure(
    confinement: Arc<Confinement>,
) -> impl FnMut() -> io::Result<()> + Send + Sync + 'static {
    move || confinement.apply()
}

fn marshal(path: impl AsRef<Path>) -> Result<std::ffi::CString, SandboxError> {
    std::ffi::CString::new(path.as_ref().as_os_str().as_encoded_bytes()).map_err(|_| {
        SandboxError::RootNotSafeForProfile {
            path: path.as_ref().display().to_string(),
            reason: "path contains NUL".to_owned(),
        }
    })
}

fn marshal_error(what: &str) -> SandboxError {
    SandboxError::RootNotSafeForProfile {
        path: what.to_owned(),
        reason: "content contains NUL".to_owned(),
    }
}
