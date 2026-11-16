use super::*;

/// Test 2 — a read outside `fs_roots` is denied, with the EPERM evidence in
/// stderr; the unsandboxed control read succeeds, so the denial is
/// attributed to the sandbox.
#[cfg(target_os = "macos")]
#[test]
fn a_read_outside_fs_roots_is_denied_with_eperm() {
    let (_root, provision) = proven_spawn("read-outside", vec![]);
    let spawn = provision.spawn().expect("proven");

    let control = bounded(Command::new("/bin/cat").arg("/private/etc/hosts"));
    assert!(
        control.status.success(),
        "the unsandboxed control read must succeed, else the deny is unattributed: \
         {control:?}"
    );

    let output = bounded(
        spawn
            .command(Path::new("/bin/cat"))
            .arg("/private/etc/hosts"),
    );
    assert!(
        !output.status.success(),
        "the sandboxed read was not denied"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Operation not permitted"),
        "the deny must carry EPERM evidence, got: {stderr}"
    );
    assert!(
        output.stdout.is_empty(),
        "the denied read must deliver nothing: {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
}

/// Test 3 — writes outside the roots are denied (EPERM, nothing on disk); a
/// write inside succeeds and the file exists afterwards, verified
/// unsandboxed.
#[cfg(target_os = "macos")]
#[test]
fn writes_outside_are_denied_inside_succeed() {
    let (root, provision) = proven_spawn("write", vec![]);
    let spawn = provision.spawn().expect("proven");
    let inside = root.canonical();
    let outside = TempRoot::new("write-outside");
    let outside_path = outside.canonical();

    let control = bounded(Command::new("/bin/mkdir").arg(outside_path.join("control-write")));
    assert!(
        control.status.success(),
        "the unsandboxed control write must succeed: {control:?}"
    );

    let denied = bounded(
        spawn
            .command(Path::new("/bin/mkdir"))
            .arg(outside_path.join("denied-write")),
    );
    assert!(
        !denied.status.success(),
        "the sandboxed write was not denied"
    );
    let stderr = String::from_utf8_lossy(&denied.stderr);
    assert!(
        stderr.contains("Operation not permitted"),
        "the deny must carry EPERM evidence, got: {stderr}"
    );
    assert!(
        !outside_path.join("denied-write").exists(),
        "a denied write must leave nothing on disk"
    );

    let allowed = bounded(
        spawn
            .command(Path::new("/bin/mkdir"))
            .arg(inside.join("after-probe-write")),
    );
    assert!(
        allowed.status.success(),
        "the sandboxed write inside fs_roots must succeed: {:?}",
        String::from_utf8_lossy(&allowed.stderr)
    );
    assert!(
        inside.join("after-probe-write").is_dir(),
        "the written directory must exist afterwards, unsandboxed"
    );
}
