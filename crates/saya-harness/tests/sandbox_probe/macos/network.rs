fn net_checks(checks: &mut Vec<Check>, root: &Path) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("probe listener");
    let port = listener.local_addr().expect("listener addr").port();
    let sb = RunSandbox {
        fs_roots: vec![root.to_path_buf()],
        net_allow: vec![("127.0.0.1".to_owned(), port)],
    };

    let mut chosen: Option<NetRule> = None;
    let mut variant_notes = String::new();
    for rule in [
        NetRule::TcpRawHost,
        NetRule::TcpLocalhostExact,
        NetRule::IpLocalhostExact,
        NetRule::TcpAnyPort,
    ] {
        let profile = seatbelt_profile(&sb, Some(rule));
        let probe = sandboxed(
            SANDBOX_EXEC,
            &profile,
            root,
            "/bin/echo",
            &["saya-net-rule-ok"],
        );
        if probe.exited_ok() {
            let _ = writeln!(
                variant_notes,
                "ACCEPTED (profile compiles) {}: {}",
                rule.name(),
                probe.summary()
            );
            if chosen.is_none() {
                chosen = Some(rule);
            }
        } else {
            let _ = writeln!(
                variant_notes,
                "REJECTED {}:\n{}",
                rule.name(),
                indent(&probe.summary())
            );
        }
    }
    checks.push(Check::info(
        "net_rule_variant_accepted",
        format!(
            "chosen: {}",
            chosen.map_or("none of the tested variants".to_owned(), |r| r
                .name()
                .to_owned())
        ) + &variant_notes,
    ));

    let control_listener = TcpListener::bind("127.0.0.1:0").expect("control listener");
    let control_port = control_listener.local_addr().expect("control addr").port();
    let control = plain_connect(control_port);
    checks.push(if control.exited_ok() {
        Check::passed("control_net_connect", true, control.summary())
    } else {
        Check::failed(
            "control_net_connect",
            true,
            format!(
                "unsandboxed connect to a loopback listener failed, so the sandboxed \
                     egress results below are unattributed:\n{}",
                control.summary()
            ),
        )
    });

    let no_net = RunSandbox {
        fs_roots: vec![root.to_path_buf()],
        net_allow: Vec::new(),
    };
    let profile_no_net = seatbelt_profile(&no_net, None);
    checks.push(expect_deny(
        &sandboxed(
            SANDBOX_EXEC,
            &profile_no_net,
            root,
            "/bin/bash",
            &["-c", &connect_script(port)],
        ),
        "net_denied_without_allow",
        true,
        None,
    ));

    checks.push(match chosen {
        None => Check::failed(
            "net_allowed_with_rule",
            false,
            format!("no tested network-allow rule syntax was accepted:\n{variant_notes}"),
        ),
        Some(rule) => {
            let profile = seatbelt_profile(&sb, Some(rule));
            let attempts = 10;
            let mut landed = 0usize;
            let mut exits_ok = 0usize;
            let mut first_denial = None;
            for _ in 0..attempts {
                let got = sandboxed(
                    SANDBOX_EXEC,
                    &profile,
                    root,
                    "/bin/bash",
                    &["-c", &connect_script(port)],
                );
                if got.exited_ok() {
                    exits_ok += 1;
                } else if first_denial.is_none() {
                    first_denial = Some(got.summary());
                }
                if receive_on_listener(&listener, Duration::from_millis(500)).is_some() {
                    landed += 1;
                }
            }
            let reliable = exits_ok == attempts && landed == attempts;
            let detail = format!(
                "rule kind {}: of {attempts} attempted connects to the allowed \
                     port, {exits_ok} exited cleanly and {landed} landed on the \
                     listener; the seatbelt allow for net_allow on this host is \
                     therefore {}.",
                rule.name(),
                if reliable {
                    "reliable (every connect landed)"
                } else {
                    "NOT reliable — selective egress is unproven, and the runner \
                         must refuse net_allow policies here"
                }
            ) + &first_denial
                .map_or_else(String::new, |s| format!("\nfirst denial:\n{}", indent(&s)));
            if reliable {
                Check::passed("net_allowed_with_rule", false, detail)
            } else {
                Check::failed("net_allowed_with_rule", false, detail)
            }
        }
    });

    checks.push(match chosen {
        None => Check::failed(
            "net_other_port_denied",
            false,
            format!(
                "no rule syntax was accepted, so port-exactness is unmeasured:\n\
                     {variant_notes}"
            ),
        ),
        Some(rule) => {
            let profile = seatbelt_profile(&sb, Some(rule));
            let attempts = 5;
            let mut denied = 0usize;
            let mut last = None;
            for _ in 0..attempts {
                let got = sandboxed(
                    SANDBOX_EXEC,
                    &profile,
                    root,
                    "/bin/bash",
                    &["-c", &connect_script(control_port)],
                );
                let was_denied = !got.exited_ok() && got.stderr.contains(EPERM_EVIDENCE);
                if was_denied {
                    denied += 1;
                }
                last = Some(got);
            }
            let detail = format!(
                "rule kind {}: connecting to a port the rule does not name was \
                     denied in {denied}/{attempts} attempts (port-exact enforcement).",
                rule.name(),
            );
            if denied == attempts {
                Check::passed("net_other_port_denied", false, detail)
            } else {
                Check::failed(
                    "net_other_port_denied",
                    false,
                    detail
                        + &last.map_or_else(String::new, |c| {
                            format!("\nlast attempt:\n{}", indent(&c.summary()))
                        }),
                )
            }
        }
    });
}
