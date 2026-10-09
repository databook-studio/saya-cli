use super::*;

/// `/recovery` through the real headless binary: the status and the toggle
/// drive the shared operation both surfaces run. Off by default; without a
/// bound workspace the status says so and `on` refuses with the launch
/// remedy (exit 5); an unknown subcommand is a usage error; a restore of
/// an id nothing recorded refuses with nothing changed (exit 5); and with
/// a workspace the toggle explains what it keeps and what it never covers,
/// with the already-on and off answers. One turn per run; stdin is null
/// throughout — the forms never prompt.
#[test]
fn turn_file_recovery_status_on_and_restore_drive_the_shared_operation() {
    let env = test_root("recovery-slash");
    let (address, _bodies) = mock("unused");
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(workspace.join(".git")).unwrap();
    let run = |line: &str, with_workspace: bool| {
        let path = env.root.join("instruction.md");
        std::fs::write(&path, format!("{line}\n")).unwrap();
        let mut args = base_args(&env);
        if with_workspace {
            args.push("--workspace".into());
            args.push(workspace.to_str().unwrap().into());
        }
        args.push("--turn-file".into());
        args.push(path.to_str().unwrap().into());
        saya(
            &env,
            &address,
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
        )
        .output()
        .unwrap()
    };

    // The status without a bound workspace: exit zero, off, and the
    // no-workspace sentence appended.
    let output = run("/recovery", false);
    assert_eq!(
        output.status.code(),
        Some(0),
        "the status completes; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        stdout.contains("Recovery is off"),
        "the status names the flag: {stdout}"
    );
    assert!(
        stdout.contains("No workspace is bound."),
        "the status says no workspace binds: {stdout}"
    );

    // `/recovery on` without a workspace refuses: exit 5, the refusal
    // names the launch remedy, and the flag stays off.
    let output = run("/recovery on", false);
    assert_eq!(
        output.status.code(),
        Some(5),
        "the refusal is an errored turn; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains(
            "No workspace is bound, so there are no file edits to record. Launch inside a \
             git worktree or pass --workspace <dir>."
        ),
        "the refusal names the remedy: {stderr}"
    );

    // An unknown subcommand: the usage is named, exit non-zero.
    let output = run("/recovery bogus", false);
    assert_ne!(
        output.status.code(),
        Some(0),
        "an unknown subcommand is a usage error; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains("unknown /recovery subcommand: bogus")
            && stderr.contains("/recovery restore <id>"),
        "the usage error names every form: {stderr}"
    );

    // A restore of an id nothing recorded: exit 5, nothing to restore,
    // nothing changed.
    let output = run("/recovery restore ck-000000", false);
    assert_eq!(
        output.status.code(),
        Some(5),
        "the restore refusal is an errored turn; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains("no recorded edit"),
        "the unknown id is named: {stderr}"
    );

    // Bound: the toggle explains what it keeps and what it never covers,
    // an already-on toggle is named, and off answers plainly.
    let output = run("/recovery on", true);
    assert_eq!(
        output.status.code(),
        Some(0),
        "the toggle completes; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        stdout.contains("Recovery is on. From the next file edit,"),
        "the retention text is said: {stdout}"
    );
    assert!(
        stdout.contains("never commands, downloads, scratch tables or databases"),
        "the retention text says what it does not cover: {stdout}"
    );
    assert!(
        stdout.contains("/recovery restore <id> to undo one"),
        "the retention text names the undo form: {stdout}"
    );

    // The off answer and the bound status with the quota line (each run is
    // a fresh session, so the flag is off again — the already-on shape is
    // pinned by the unit and chain tests).
    let output = run("/recovery off", true);
    assert_eq!(
        output.status.code(),
        Some(0),
        "the off completes; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        stdout.contains("Recovery is off."),
        "the off answer is said: {stdout}"
    );

    let output = run("/recovery", true);
    assert_eq!(
        output.status.code(),
        Some(0),
        "the bound status completes; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        stdout.contains(
            "Recovery is off. 0 recorded edit(s) this session (0 KiB of 64 MiB, 0 of 128)."
        ),
        "the status carries the quota line: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}
