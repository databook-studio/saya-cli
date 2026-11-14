#[test]
fn scripted_repl_persists_and_continues_a_redacted_session() {
    use std::io::Write;
    let root = std::env::temp_dir().join(format!("saya-cli-repl-{}", std::process::id()));
    let sessions = root.join("sessions");
    let mut child = super::common::saya_command(&root, &["--format", "ndjson"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"/help\n/exit\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("/help"));
    assert!(std::fs::read_dir(&sessions).unwrap().any(|entry| {
        entry
            .unwrap()
            .path()
            .extension()
            .is_some_and(|ext| ext == "json")
    }));
    let resumed = super::common::saya_command(&root, &["--continue"])
        .output()
        .unwrap();
    assert!(resumed.status.success());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn non_interactive_mode_does_not_start_a_prompt() {
    let root = std::env::temp_dir().join(format!("saya-cli-noninteractive-{}", std::process::id()));
    let output = super::common::saya_command(&root, &["--non-interactive"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires a subcommand"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn non_interactive_defaults_to_never_approval_but_interactive_defaults_to_ask() {
    let interactive = GlobalOptions::default();
    let non_interactive = GlobalOptions {
        non_interactive: true,
        ..Default::default()
    };
    assert_eq!(approval_name(&interactive).unwrap(), "ask");
    assert_eq!(approval_name(&non_interactive).unwrap(), "never");
    let explicit = GlobalOptions {
        non_interactive: true,
        approval_mode: Some("read-only".into()),
        ..Default::default()
    };
    assert_eq!(approval_name(&explicit).unwrap(), "read-only");
}

/// A piped interactive run has no operator surface: the startup composition
/// notice — the plain-folder binding, or the Git-root in-place coding fact —
/// belongs to the TUI transcript or a terminal-attached prompt, never to a
/// scripted run's stderr.
#[test]
fn piped_interactive_run_prints_no_workspace_notice_on_stderr() {
    use std::io::Write;
    let root = super::common::test_root("piped-notice");
    let mut child = super::common::saya_command(&root, &["--format", "ndjson"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"/exit\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let stderr = super::common::stderr(&output);
    assert!(
        !stderr.contains("in place") && !stderr.contains("No workspace is bound"),
        "a piped run says no startup notice on stderr: {stderr:?}"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn idle_eof_exits_the_interactive_process_cleanly() {
    let root = std::env::temp_dir().join(format!("saya-cli-eof-{}", std::process::id()));
    let child = super::common::saya_command(&root, &[])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let _ = std::fs::remove_dir_all(root);
}
