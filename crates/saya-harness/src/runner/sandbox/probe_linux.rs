//! The Linux startup canary battery: measures the Landlock ABI, the
//! unprivileged user+network namespace, and — through the real
//! [`Confinement`](super::linux::Confinement) code — whether the confinement
//! actually denies. The spike's Linux half recorded `landlock_enforcement`
//! as failed *by design* (the spike could not add the enforcement probe);
//! this is the implementation slice's canary. It runs on a Linux host
//! through exactly the code [`RunnerSpawn`](super::spawn::RunnerSpawn) will
//! use, and the verdict reads its result — never the assumptions in
//! `linux.rs`.
//!
//! Required checks (draft §5): `landlock_abi`,
//! `user_network_namespace_unprivileged`, `landlock_enforcement`; plus
//! `namespace_egress_denied` and `confinement_exec_allowed` (see
//! `linux_canary`). Deny evidence on Linux is the errno — EACCES or EPERM
//! for the filesystem, and for egress any failure that is not a plain
//! connection-refused [UNVERIFIED which errno each surface returns; the
//! probe records what it measured].

use std::{fs, path::PathBuf};

use super::{
    RunSandbox,
    linux::{Confinement, MIN_LANDLOCK_ABI},
    linux_canary::{confinement_exec_allowed, landlock_enforcement_check, namespace_egress_check},
    linux_fork::{collect_canary, fork_with_pipe, raw_io, write_words},
    probe_support::uname_line,
    report::{Check, ProbeReport},
};

/// The canary programs' directory: the exec canary execs a binary from here,
/// the same directory shape the runner allowlists.
pub(super) const CANARY_PROGRAM_DIR: &str = "/bin";

pub(super) fn canary_program_dir() -> PathBuf {
    PathBuf::from(CANARY_PROGRAM_DIR)
}

/// A temp directory for the canaries' outside-root targets; removed on drop.
struct TempRoot(PathBuf);

impl TempRoot {
    fn new(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("saya-sbx-probe-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("probe temp root must be creatable");
        Self(path)
    }

    fn canonical(&self) -> PathBuf {
        fs::canonicalize(&self.0).expect("probe temp root must canonicalise")
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn errno_text(errno: i32) -> String {
    std::io::Error::from_raw_os_error(errno).to_string()
}

fn read_record(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| format!("absent ({e})"))
}

/// The `unshare(CLONE_NEWUSER)` + `unshare(CLONE_NEWNET)` availability
/// check, promoted in shape from the spike: the first failure's errno is the
/// recorded evidence. Slots: [user, net].
fn unprivileged_user_net_ns() -> Result<(), String> {
    let (pid, fd) = fork_with_pipe(|fds| {
        let mut w = [0_i32; 4];
        if unsafe { libc::unshare(libc::CLONE_NEWUSER) } != 0 {
            w[0] = -raw_io();
        } else {
            w[0] = 0;
            if unsafe { libc::unshare(libc::CLONE_NEWNET) } != 0 {
                w[1] = -raw_io();
            }
        }
        write_words(fds[1], &w);
    })?;
    let result = collect_canary(pid, fd);
    let user = result.slot(0);
    let net = result.slot(1);
    if !result.exited() {
        return Err("the namespace probe child was killed at the wall bound".into());
    }
    if user == 0 && net == 0 {
        Ok(())
    } else if user != 0 && user != i32::MIN {
        Err(format!(
            "unshare(CLONE_NEWUSER) failed with errno {}: {} — the netns egress \
             mechanism is unavailable on this host",
            -user,
            errno_text(-user)
        ))
    } else {
        Err(format!(
            "unshare(CLONE_NEWNET) failed with errno {}: {} — the netns egress \
             mechanism is unavailable on this host",
            -net,
            errno_text(-net)
        ))
    }
}

pub(super) fn run(sb: &RunSandbox) -> ProbeReport {
    let uname = uname_line();
    let mut checks = vec![Check::info("host_uname", uname.clone())];

    checks.push(match super::linux_sys::landlock_abi() {
        Ok(v) if v >= MIN_LANDLOCK_ABI => Check::pass(
            "landlock_abi",
            true,
            format!(
                "landlock_create_ruleset(NULL, 0, LANDLOCK_CREATE_RULESET_VERSION) returned \
                 ABI {v}; this design requires ≥ {MIN_LANDLOCK_ABI} (TRUNCATE)"
            ),
        ),
        Ok(v) => Check::fail(
            "landlock_abi",
            true,
            format!(
                "Landlock ABI {v} is below this design's minimum ({MIN_LANDLOCK_ABI}); \
                 TRUNCATE cannot be handled, so the ruleset would silently under-restrict"
            ),
        ),
        Err(e) => Check::fail(
            "landlock_abi",
            true,
            format!(
                "Landlock version query failed ({e}): this kernel does not expose Landlock \
                 to unprivileged callers"
            ),
        ),
    });

    let lsm = read_record("/sys/kernel/security/lsm");
    checks.push(Check::info(
        "lsm_list",
        format!(
            "landlock listed in /sys/kernel/security/lsm: {}\ncontent: {}",
            lsm.contains("landlock"),
            lsm.trim_end()
        ),
    ));
    checks.push(Check::info(
        "user_max_user_namespaces",
        read_record("/proc/sys/user/max_user_namespaces")
            .trim_end()
            .to_owned(),
    ));

    checks.push(match unprivileged_user_net_ns() {
        Ok(()) => Check::pass(
            "user_network_namespace_unprivileged",
            true,
            "a forked, unprivileged child ran unshare(CLONE_NEWUSER) then \
             unshare(CLONE_NEWNET); both succeeded",
        ),
        Err(e) => Check::fail("user_network_namespace_unprivileged", true, e),
    });

    let outside = TempRoot::new("outside");
    let outside_path = outside.canonical();
    let inside = sb.fs_roots()[0].clone();

    match Confinement::new(sb, &[canary_program_dir()]) {
        Err(e) => checks.push(Check::fail(
            "landlock_enforcement",
            true,
            format!("the confinement could not be built on this kernel: {e}"),
        )),
        Ok(conf) => {
            let conf = std::sync::Arc::new(conf);
            checks.push(landlock_enforcement_check(&conf, &inside, &outside_path));
            checks.push(namespace_egress_check(&conf, &inside, &outside_path));
            checks.push(confinement_exec_allowed(&conf));
        }
    }

    ProbeReport::new("linux", uname, checks)
}
