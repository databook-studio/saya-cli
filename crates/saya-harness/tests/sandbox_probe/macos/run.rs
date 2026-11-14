pub(super) fn run() -> ProbeReport {
    let uname = uname_line();
    let mut checks = vec![Check::info("host_uname", uname.clone())];
    let run_root = TempRoot::new("run-root");
    let outside_root = TempRoot::new("outside-root");
    let root = run_root.canonical();
    let outside = outside_root.canonical();

    checks.push(if Path::new(SANDBOX_EXEC).exists() {
        Check::passed(
            "sandbox_exec_present",
            true,
            format!("{SANDBOX_EXEC} exists on this host"),
        )
    } else {
        Check::failed(
            "sandbox_exec_present",
            true,
            format!("{SANDBOX_EXEC} missing; no Seatbelt path on this host"),
        )
    });

    let bogus = sandboxed(
        SANDBOX_EXEC,
        "not a sandbox profile at all",
        &root,
        "/bin/echo",
        &["saya-probe-bogus"],
    );
    checks.push(if !bogus.exited_ok() {
        Check::passed("bogus_profile_rejected", true, bogus.summary())
    } else {
        Check::failed(
            "bogus_profile_rejected",
            true,
            format!(
                "sandbox-exec accepted an invalid profile — it cannot be trusted to \
                     parse profiles at all:\n{}",
                bogus.summary()
            ),
        )
    });

    checks.push(expect_deny(
        &sandboxed(
            SANDBOX_EXEC,
            "(version 1)\n(deny default)\n",
            &root,
            "/bin/echo",
            &["x"],
        ),
        "deny_default_denies_exec",
        true,
        None,
    ));

    let control_write = plain(
        "/bin/mkdir",
        &[&outside.join("control-write").to_string_lossy()],
    );
    checks.push(if control_write.exited_ok() {
        Check::passed("control_write_outside_root", true, control_write.summary())
    } else {
        Check::failed(
            "control_write_outside_root",
            true,
            format!(
                "unsandboxed mkdir into the outside dir failed, so the sandboxed \
                     write-deny result below is unattributable:\n{}",
                control_write.summary()
            ),
        )
    });

    let profile_no_net = seatbelt_profile(
        &RunSandbox {
            fs_roots: vec![root.clone()],
            net_allow: Vec::new(),
        },
        None,
    );
    checks.push(expect_deny(
        &sandboxed(
            SANDBOX_EXEC,
            &profile_no_net,
            &root,
            "/bin/mkdir",
            &[&outside.join("denied-write").to_string_lossy()],
        ),
        "write_denied_outside_root",
        true,
        Some(&outside.join("denied-write")),
    ));

    let allowed = sandboxed(
        SANDBOX_EXEC,
        &profile_no_net,
        &root,
        "/bin/mkdir",
        &[&root.join("allowed-write").to_string_lossy()],
    );
    let dir_created = root.join("allowed-write").is_dir();
    checks.push(if allowed.exited_ok() && dir_created {
        Check::passed(
            "write_allowed_in_root",
            true,
            format!(
                "sandboxed mkdir inside fs_roots worked; the directory exists \
                     unsandboxed afterwards:\n{}",
                allowed.summary()
            ),
        )
    } else {
        Check::failed(
            "write_allowed_in_root",
            true,
            format!(
                "sandboxed write inside fs_roots did not work (directory exists: \
                     {dir_created}):\n{}",
                allowed.summary()
            ),
        )
    });

    let control_read = plain("/bin/cat", &["/private/etc/hosts"]);
    checks.push(if control_read.exited_ok() {
        Check::passed("control_read_outside_roots", true, control_read.summary())
    } else {
        Check::failed(
            "control_read_outside_roots",
            true,
            format!(
                "unsandboxed cat of /private/etc/hosts failed, so the sandboxed \
                     read-deny result below is unattributable:\n{}",
                control_read.summary()
            ),
        )
    });
    checks.push(expect_deny(
        &sandboxed(
            SANDBOX_EXEC,
            &profile_no_net,
            &root,
            "/bin/cat",
            &["/private/etc/hosts"],
        ),
        "read_denied_outside_roots",
        true,
        None,
    ));

    net_checks(&mut checks, &root);

    ProbeReport {
        platform: "macos",
        kernel: uname,
        checks,
    }
}
