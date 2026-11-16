/// `saya preferences --help` states the contract: explicit, per-worktree
/// guidance, never permissions.
#[test]
fn help_mentions_never_permissions() {
    let root = unique_dir("help");
    let help = run(&root, &root, &["preferences", "--help"]);
    assert_eq!(help.status.code(), Some(0), "{}", stderr_of(&help));
    let out = stdout_of(&help);
    assert!(out.contains("never permissions"), "{out}");
    for subcommand in ["list", "add", "edit", "forget", "forget-all"] {
        assert!(out.contains(subcommand), "help names {subcommand}: {out}");
    }
    cleanup(&root);
}

/// Independence smoke: `saya --help` still presents the whole surface
/// (typed database knowledge under `contracts` and preferences beside it),
/// with no database profile needed anywhere in this file.
#[test]
fn saya_help_smoke_shows_both_surfaces() {
    let root = unique_dir("smoke");
    let help = run(&root, &root, &["--help"]);
    assert_eq!(help.status.code(), Some(0), "{}", stderr_of(&help));
    let out = stdout_of(&help);
    assert!(out.contains("preferences"), "{out}");
    assert!(out.contains("contracts"), "{out}");
    cleanup(&root);
}

/// The command never reads stdin: with an open, empty pipe held open, a
/// `preferences list` completes on its own instead of blocking forever.
#[test]
fn the_command_never_reads_stdin() {
    let root = unique_dir("no-stdin");
    let wt = fake_worktree(&root, "wt");
    let mut child = saya(&root, &wt, &["preferences", "list"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    // Held open and never written: any stdin read would block forever.
    let _held_open = child.stdin.take();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the command blocked waiting on stdin"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(status.code(), Some(0), "list completed without stdin");
    let mut stdout = String::new();
    use std::io::Read;
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    assert!(stdout.contains("No project preferences for "), "{stdout}");
    cleanup(&root);
}
