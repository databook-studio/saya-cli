/// The session's `shell` is the shared command tool, advertised wherever the
/// mode keeps a lane for it: `ask` (through this turn's approval surface, or
/// from the sandbox when the surface is headless), `bypass`, and `read-only`
/// — whose only lane is the write-denied sandbox, where nothing in the
/// checkout is writable. That lane exists only where the host can express a
/// denial: macOS's seatbelt can, Linux's allow-only Landlock cannot, so the
/// read-only row follows this host's own `prove` answer. `never` keeps no
/// lane, and Plan hides the shell with the rest of the write-shaped surface,
/// whose posture denies an external side effect.
#[test]
fn the_session_advertises_the_shell_in_every_mode_but_never() {
    let project = worktree("shell-advertise");
    let state = temp_dir("shell-advertise-state");
    let universe = compose(&session_runtime(), &project, &state);
    let read_only_shell = read_only_sandbox_proves(&project);
    let ask_headless_shell = ask_sandbox_proves(&project);
    for (mode, can_obtain_approval, expected) in [
        (ApprovalPolicy::Ask, true, true),
        (ApprovalPolicy::Ask, false, ask_headless_shell),
        (ApprovalPolicy::Bypass, false, true),
        (ApprovalPolicy::ReadOnly, true, read_only_shell),
    ] {
        let names = advertised(&universe, AgentMode::Build, mode, can_obtain_approval);
        assert_eq!(
            names.contains(&"shell".to_string()),
            expected,
            "the session advertises the shell under {mode:?} \
             (can_obtain_approval={can_obtain_approval}): {names:?}"
        );
    }
    let never = advertised(&universe, AgentMode::Build, ApprovalPolicy::Never, true);
    assert!(
        !never.contains(&"shell".to_string()),
        "`never` keeps no lane, so the shell is absent: {never:?}"
    );
    // Plan hides it: the shell declares an external side effect, and Plan's
    // posture denies those — an advertised-but-always-refused tool is the
    // anti-pattern the hidden-not-advertised rule exists to prevent.
    let plan = advertised(&universe, AgentMode::Plan, ApprovalPolicy::Ask, true);
    assert!(
        !plan.contains(&"shell".to_string()),
        "Plan hides the shell with the write-shaped surface: {plan:?}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}
