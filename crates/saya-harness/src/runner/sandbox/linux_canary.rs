//! The Linux enforcement canary checks: the filesystem deny, the netns
//! egress deny, and the exec allow — each through the real
//! [`Confinement`](super::linux::Confinement) code the runner will use, with
//! the errno as the deny evidence.
//!
//! Required for the verdict (draft §5): `landlock_enforcement`; plus
//! `namespace_egress_denied` (the egress mechanism itself proven, not
//! assumed) and `confinement_exec_allowed` (the canary child can exec inside
//! its program dir). Deny evidence on Linux is the errno — EACCES or EPERM
//! for the filesystem, and for egress any failure that is not a plain
//! connection-refused [UNVERIFIED which errno each surface returns; the
//! probe records what it measured].

use std::{path::Path, sync::Arc};

use super::linux::Confinement;
use super::linux_fork::{CanaryResult, fork_exec_canary, fork_fs_canary};
use super::report::Check;

/// The filesystem enforcement canary: a forked child applies the real
/// confinement, then must succeed writing inside the root and fail creating
/// outside it. Deny evidence is the errno — EACCES or EPERM [UNVERIFIED
/// which surface yields which].
pub(super) fn landlock_enforcement_check(
    conf: &Arc<Confinement>,
    inside: &Path,
    outside: &Path,
) -> Check {
    match fork_fs_canary(Arc::clone(conf), inside, outside, 9) {
        Err(e) => Check::fail(
            "landlock_enforcement",
            true,
            format!("the enforcement canary could not run: {e}"),
        ),
        Ok(result) => {
            let apply = result.slot(0);
            let inside_mkdir = result.slot(1);
            let outside_create = result.slot(2);
            let apply_ok = apply == 0;
            let inside_ok = inside_mkdir == 0;
            let outside_denied = -outside_create == libc::EACCES || -outside_create == libc::EPERM;
            if !result.exited() {
                Check::fail(
                    "landlock_enforcement",
                    true,
                    "the enforcement canary child was killed at the wall bound".to_owned(),
                )
            } else if apply_ok && inside_ok && outside_denied {
                Check::pass(
                    "landlock_enforcement",
                    true,
                    format!(
                        "the confined canary wrote inside fs_roots (succeeded) and created \
                         outside the roots (denied: errno {} — {}) — Landlock enforcement \
                         proven on this kernel",
                        -outside_create,
                        errno_text(-outside_create)
                    ),
                )
            } else if !apply_ok {
                Check::fail(
                    "landlock_enforcement",
                    true,
                    format!(
                        "the confinement itself failed in the canary child (errno {}: {}) — \
                         fail closed",
                        -apply,
                        if apply == i32::MIN {
                            "no report".to_owned()
                        } else {
                            errno_text(-apply)
                        }
                    ),
                )
            } else if !inside_ok {
                Check::fail(
                    "landlock_enforcement",
                    true,
                    format!(
                        "the confined canary could not write inside fs_roots (errno {}: {}) \
                         — the confinement is too tight to be the runner's, fail closed",
                        -inside_mkdir,
                        errno_text(-inside_mkdir)
                    ),
                )
            } else {
                Check::fail(
                    "landlock_enforcement",
                    true,
                    format!(
                        "the confined canary's create outside the roots returned errno {} — \
                         {} — Landlock did not enforce the deny",
                        -outside_create,
                        if outside_create == 0 {
                            "the create SUCCEEDED".to_owned()
                        } else {
                            errno_text(-outside_create)
                        }
                    ),
                )
            }
        }
    }
}

/// The egress mechanism itself, proven not assumed: the confined canary
/// child's loopback connect must fail — any errno except a plain
/// connection-refused [UNVERIFIED which errno the netns yields; recorded].
pub(super) fn namespace_egress_check(
    conf: &Arc<Confinement>,
    inside: &Path,
    outside: &Path,
) -> Check {
    match fork_fs_canary(Arc::clone(conf), inside, outside, 9) {
        Err(e) => Check::fail(
            "namespace_egress_denied",
            true,
            format!("the egress canary could not run: {e}"),
        ),
        Ok(result) => {
            let connect = result.slot(3);
            if !result.exited() {
                Check::fail(
                    "namespace_egress_denied",
                    true,
                    "the egress canary child was killed at the wall bound".to_owned(),
                )
            } else if result.slot(0) != 0 {
                Check::fail(
                    "namespace_egress_denied",
                    true,
                    "the confinement failed before the connect could be measured".to_owned(),
                )
            } else if connect == 0 {
                Check::fail(
                    "namespace_egress_denied",
                    true,
                    "the confined canary's loopback connect SUCCEEDED — the netns did not \
                     isolate egress"
                        .to_owned(),
                )
            } else if -connect == libc::ECONNREFUSED {
                Check::fail(
                    "namespace_egress_denied",
                    true,
                    "the confined canary's connect was refused the way an unsandboxed \
                     connect to a closed port is — no netns evidence"
                        .to_owned(),
                )
            } else {
                Check::pass(
                    "namespace_egress_denied",
                    true,
                    format!(
                        "the confined canary's loopback connect failed with errno {} ({}) — \
                         the netns denies egress on this host",
                        -connect,
                        errno_text(-connect)
                    ),
                )
            }
        }
    }
}

/// The exec canary: the confined child must be able to exec a canary binary
/// from its program directory — the runner's whole purpose. Success is the
/// binary's own clean exit.
pub(super) fn confinement_exec_allowed(conf: &Arc<Confinement>) -> Check {
    let program = super::probe_linux::canary_program_dir().join("true");
    match fork_exec_canary(Arc::clone(conf), &program) {
        Err(e) => Check::fail(
            "confinement_exec_allowed",
            true,
            format!("the exec canary could not run: {e}"),
        ),
        Ok(result) => {
            let apply = result.slot(0);
            let exec = result.slot(1);
            judge_exec(&result, &program, apply, exec)
        }
    }
}

fn judge_exec(result: &CanaryResult, program: &Path, apply: i32, exec: i32) -> Check {
    if !result.exited() {
        Check::fail(
            "confinement_exec_allowed",
            true,
            "the exec canary child was killed at the wall bound".to_owned(),
        )
    } else if apply != 0 && apply != i32::MIN {
        Check::fail(
            "confinement_exec_allowed",
            true,
            format!(
                "the confinement failed before the exec (errno {}: {}) — fail closed",
                -apply,
                errno_text(-apply)
            ),
        )
    } else if exec != 0 && exec != i32::MIN {
        Check::fail(
            "confinement_exec_allowed",
            true,
            format!(
                "the confined canary could not exec {} (errno {}: {}) — the program-dir \
                 allow or the loader paths are wrong for this host, fail closed",
                program.display(),
                -exec,
                errno_text(-exec)
            ),
        )
    } else if result.code() == Some(0) {
        Check::pass(
            "confinement_exec_allowed",
            true,
            format!(
                "the confined canary exec'd {} and it exited cleanly",
                program.display()
            ),
        )
    } else {
        Check::fail(
            "confinement_exec_allowed",
            true,
            format!(
                "the exec canary did not exit cleanly (code {:?}); the cause is \
                 unattributed",
                result.code()
            ),
        )
    }
}

fn errno_text(errno: i32) -> String {
    std::io::Error::from_raw_os_error(errno).to_string()
}
