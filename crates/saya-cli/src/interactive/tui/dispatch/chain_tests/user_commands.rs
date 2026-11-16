/// A user-authored command reaches the agent as its expanded prompt — never
/// through the built-in parser — and a refused file says why in the
/// transcript instead of running anything. The TUI surface of
/// `slash::user_commands`.
#[test]
fn a_user_command_dispatches_as_the_expanded_prompt() {
    let mut fx = ChainFixture::build("user-command");
    let dir = fx.root.0.join("commands");
    std::fs::create_dir_all(&dir).expect("the commands directory");
    std::fs::write(
        dir.join("review.md"),
        "---\ndescription: Review a diff\n---\nReview $ARGUMENTS.\n",
    )
    .expect("the command file");
    std::fs::write(dir.join("big.md"), "x".repeat(9 * 1024)).expect("the oversized file");
    fx.state.user_commands = crate::slash::user_commands::load(&dir, None);
    match dispatch_line("/review src/lib.rs", &mut fx) {
        Dispatch::Agent(prompt) => assert_eq!(prompt, "Review src/lib.rs."),
        _ => panic!("a user command is a prompt for the agent, never a built-in"),
    }
    assert!(
        matches!(dispatch_line("/big", &mut fx), Dispatch::Handled),
        "a refused file is handled as an error, never sent to the agent"
    );
    assert!(
        fx.app
            .transcript
            .blocks()
            .iter()
            .any(|block| block.kind == BlockKind::Error
                && block.text.contains("over the 8192-byte limit")),
        "the refusal is said in the transcript"
    );
    assert!(
        matches!(dispatch_line("/mode plan", &mut fx), Dispatch::Handled),
        "the built-ins still parse"
    );
}
