//! The macOS startup canary battery: proves the generated Seatbelt profile
//! confines children on this host, at this moment. Required checks (fail
//! closed — all must pass for [`ProbeReport::proves_runner`]):
//!
//! - `sandbox_exec_present` — the measured sandbox-exec path exists;
//! - `bogus_profile_rejected` — sandbox-exec rejects an invalid profile (a
//!   no-op sandbox-exec would make every later check meaningless);
//! - `deny_default_denies_exec` — `(deny default)` alone refuses exec;
//! - `profile_parses_and_runs` — the real generated probe profile parses and
//!   runs its allowlisted child;
//! - `control_write_outside_root` + `write_denied_outside_root` — the same
//!   write succeeds unsandboxed and is denied sandboxed with `Operation not
//!   permitted` and nothing on disk;
//! - `write_allowed_inside_root` — the sandboxed child writes inside
//!   `fs_roots` and the file exists afterwards, verified unsandboxed;
//! - `control_read_outside_roots` + `read_denied_outside_roots`;
//! - `control_net_connect` + `egress_denied_other_port` (EPERM) +
//!   `egress_allowed_endpoint` — every attempted connect to the allowed port
//!   must land on a listener that actually calls `accept()` (the spike's
//!   starved-listener lesson: one that never accepts turns timeouts into
//!   false denials).
//!
//! The canary programs are the spike's (`/bin/echo`, `/bin/mkdir`,
//! `/bin/cat`, `/bin/bash`, all single-command — measured never to need
//! `process-fork`), so the probe profile is the real generator output plus
//! only the canary program directory.

use std::path::{Path, PathBuf};

use super::{
    RunSandbox,
    macos::SANDBOX_EXEC,
    macos_canary::{
        CANARY_PROGRAM_DIR, ProbeListener, TempRoot, expect_deny, plain, sandboxed, unique,
    },
    probe_support::{check_exited_ok, uname_line},
    report::{Check, ProbeReport},
};

/// The probe policy: the real policy, widened only by the probe listener's
/// port (appended to `net_allow` so the allowed endpoint is provable against
/// a real accepting listener).
fn probe_policy(sb: &RunSandbox, listener_port: u16) -> Result<RunSandbox, String> {
    let mut net = sb.net_allow().to_vec();
    net.push(("127.0.0.1".to_owned(), listener_port));
    RunSandbox::new(sb.fs_roots().to_vec(), net).map_err(|e| e.to_string())
}

pub(super) fn run(sb: &RunSandbox) -> ProbeReport {
    let uname = uname_line();
    let mut checks = vec![Check::info("host_uname", uname.clone())];
    let outside = TempRoot::new("outside");
    let outside_path = outside.canonical();
    let root = sb.fs_roots()[0].clone();

    let listener = match ProbeListener::bind() {
        Ok(l) => match probe_policy(sb, l.port) {
            Ok(policy) => (policy, l),
            Err(e) => {
                return ProbeReport::new(
                    "macos",
                    uname,
                    vec![Check::fail(
                        "probe_setup",
                        true,
                        format!("the probe policy could not be built: {e}"),
                    )],
                );
            }
        },
        Err(e) => {
            return ProbeReport::new(
                "macos",
                uname,
                vec![Check::fail(
                    "probe_setup",
                    true,
                    format!("the probe listener could not be bound: {e}"),
                )],
            );
        }
    };
    let (probe_sb, listener) = listener;
    let profile =
        match super::macos::seatbelt_profile(&probe_sb, &[PathBuf::from(CANARY_PROGRAM_DIR)]) {
            Ok(p) => p,
            Err(e) => {
                return ProbeReport::new(
                    "macos",
                    uname,
                    vec![Check::fail(
                        "profile_generation",
                        true,
                        format!("the generated profile was refused by the generator: {e}"),
                    )],
                );
            }
        };

    checks.push(if Path::new(SANDBOX_EXEC).exists() {
        Check::pass(
            "sandbox_exec_present",
            true,
            format!("{SANDBOX_EXEC} exists on this host"),
        )
    } else {
        Check::fail(
            "sandbox_exec_present",
            true,
            format!("{SANDBOX_EXEC} missing; no Seatbelt path on this host"),
        )
    });

    let bogus = sandboxed(
        "not a sandbox profile at all",
        &root,
        "/bin/echo",
        &["saya-probe-bogus"],
    );
    checks.push(if !bogus.exited_ok() {
        Check::pass("bogus_profile_rejected", true, bogus.summary())
    } else {
        Check::fail(
            "bogus_profile_rejected",
            true,
            format!(
                "sandbox-exec accepted an invalid profile — it cannot be trusted to parse \
                 profiles at all:\n{}",
                bogus.summary()
            ),
        )
    });

    checks.push(expect_deny(
        &sandboxed("(version 1)\n(deny default)\n", &root, "/bin/echo", &["x"]),
        "deny_default_denies_exec",
        true,
        None,
    ));

    checks.push(check_exited_ok(
        &sandboxed(&profile, &root, "/bin/echo", &["saya-probe-profile-ok"]),
        "profile_parses_and_runs",
        true,
        |c| {
            format!(
                "the generated probe profile ran its canary: stdout {:?}",
                c.stdout.trim()
            )
        },
    ));

    checks.push(check_exited_ok(
        &plain(
            "/bin/mkdir",
            &[&outside_path.join(unique("control-write")).to_string_lossy()],
        ),
        "control_write_outside_root",
        true,
        |_| "unsandboxed mkdir into the outside dir succeeded".to_owned(),
    ));

    let denied_target = outside_path.join(unique("denied-write"));
    checks.push(expect_deny(
        &sandboxed(
            &profile,
            &root,
            "/bin/mkdir",
            &[&denied_target.to_string_lossy()],
        ),
        "write_denied_outside_root",
        true,
        Some(&denied_target),
    ));

    let inside_target = root.join(unique("allowed-write"));
    let allowed = sandboxed(
        &profile,
        &root,
        "/bin/mkdir",
        &[&inside_target.to_string_lossy()],
    );
    let created = inside_target.is_dir();
    checks.push(if allowed.exited_ok() && created {
        Check::pass(
            "write_allowed_inside_root",
            true,
            format!(
                "sandboxed mkdir inside fs_roots worked; the directory exists \
                 unsandboxed afterwards:\n{}",
                allowed.summary()
            ),
        )
    } else {
        Check::fail(
            "write_allowed_inside_root",
            true,
            format!(
                "sandboxed write inside fs_roots did not work (directory exists: \
                 {created}):\n{}",
                allowed.summary()
            ),
        )
    });

    checks.push(check_exited_ok(
        &plain("/bin/cat", &["/private/etc/hosts"]),
        "control_read_outside_roots",
        true,
        |_| "unsandboxed cat of /private/etc/hosts succeeded".to_owned(),
    ));
    checks.push(expect_deny(
        &sandboxed(&profile, &root, "/bin/cat", &["/private/etc/hosts"]),
        "read_denied_outside_roots",
        true,
        None,
    ));

    super::macos_canary::egress_checks(&mut checks, sb, &profile, &root, &listener);

    ProbeReport::new("macos", uname, checks)
}
