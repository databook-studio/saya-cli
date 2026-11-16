use super::*;

/// Test 4 — egress to a port not in `net_allow` is denied with EPERM; the
/// allowed port connects, and every landing is observed on a listener that
/// calls `accept()`.
#[cfg(target_os = "macos")]
#[test]
fn egress_to_unallowed_port_is_denied_allowed_port_connects() {
    let allowed = AcceptingListener::bind();
    let control_listener = AcceptingListener::bind();
    let (_root, provision) = proven_spawn("egress", vec![("127.0.0.1".to_owned(), allowed.port)]);
    let spawn = provision.spawn().expect("proven");
    let connect = |port: u16| format!("exec 3<>/dev/tcp/127.0.0.1/{port} && printf ping >&3");

    // Control: the same connect unsandboxed must land on the accepting
    // listener, so the sandboxed results below are attributed.
    let control = bounded(
        Command::new("/bin/bash")
            .arg("-c")
            .arg(connect(control_listener.port)),
    );
    assert!(
        control.status.success(),
        "the unsandboxed control connect must succeed: {control:?}"
    );
    assert_eq!(
        control_listener.receive(Duration::from_secs(2)).as_deref(),
        Some("ping"),
        "the control connect must land on the accepting listener"
    );

    // Allowed endpoint: connects, and the listener actually accepts.
    let attempt = bounded(
        spawn
            .command(Path::new("/bin/bash"))
            .arg("-c")
            .arg(connect(allowed.port)),
    );
    assert!(
        attempt.status.success(),
        "the sandboxed connect to the allowed port must succeed: {:?}",
        String::from_utf8_lossy(&attempt.stderr)
    );
    assert_eq!(
        allowed.receive(Duration::from_secs(2)),
        Some("ping".to_owned()),
        "the allowed connect must land on the accepting listener"
    );

    // Denied endpoint: a port no rule names is refused with EPERM — the
    // literal text, never a timeout mistaken for a denial.
    let denied = bounded(
        spawn
            .command(Path::new("/bin/bash"))
            .arg("-c")
            .arg(connect(control_listener.port)),
    );
    assert!(
        !denied.status.success(),
        "the sandboxed connect was not denied"
    );
    let stderr = String::from_utf8_lossy(&denied.stderr);
    assert!(
        stderr.contains("Operation not permitted"),
        "the egress deny must carry EPERM evidence, got: {stderr}"
    );
}
