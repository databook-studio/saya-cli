use super::*;

// ---------------------------------------------------------------------------
// 2. trust_binds_exactly_the_named_dir_never_a_parent
// ---------------------------------------------------------------------------

/// Trust binds exactly the directory named — the cwd or the typed dir —
/// never a parent, never a walk. A prompt that inferred upward would claim
/// `~` from a home launch and `/` from a root launch.
#[test]
fn trust_binds_exactly_the_named_dir_never_a_parent() {
    let parent = temp_dir("exact-parent");
    let child = parent.join("child");
    fs::create_dir_all(&child).unwrap();
    // The typed dir resolves to itself, canonically — no walk-up step.
    let resolved = resolve_trusted_dir(&child).expect("the named dir resolves");
    assert_eq!(
        resolved,
        std::fs::canonicalize(&child).unwrap(),
        "the named dir binds exactly, never a parent"
    );
    assert!(
        !parent.canonicalize().unwrap().starts_with(&resolved)
            || resolved == std::fs::canonicalize(&child).unwrap(),
        "the parent is never substituted: {resolved:?}"
    );
    // End to end: binding the named child pins the child, not the parent.
    let runtime = session_runtime();
    let state = temp_dir("exact-parent-state");
    let universe = SessionUniverse::compose(&runtime, Some(&child), None, true, &child, &state)
        .expect("the named dir binds");
    assert_eq!(
        universe.root().expect("the named dir binds"),
        std::fs::canonicalize(&child).unwrap().as_path(),
        "the session root is the named dir, not the parent"
    );
    // The `w` answer carries the typed dir verbatim.
    let answer = parse_trust_answer("w /tmp/some-dir").expect("the workspace answer parses");
    assert!(
        matches!(&answer, TrustAnswer::Workspace(dir) if dir == Path::new("/tmp/some-dir")),
        "the workspace answer binds the typed dir exactly: {answer:?}"
    );
    let _ = (fs::remove_dir_all(&parent), fs::remove_dir_all(&state));
}

// ---------------------------------------------------------------------------
// 3. non_terminal_unbound_never_prompts
// ---------------------------------------------------------------------------

/// Piped stdin, `--turn-file`, `--format json` — nobody to prompt. Nothing
/// binds, exactly as today, plus G1's notice. Never hang waiting for input
/// on a surface that has none.
#[test]
fn non_terminal_unbound_never_prompts() {
    for (label, ctx) in [
        (
            "piped stdin",
            TrustPromptContext {
                is_terminal: false,
                ..unbound_terminal()
            },
        ),
        (
            "--turn-file",
            TrustPromptContext {
                is_terminal: false,
                turn_file: true,
                ..unbound_terminal()
            },
        ),
        (
            "--format json",
            TrustPromptContext {
                is_terminal: false,
                ..unbound_terminal()
            },
        ),
    ] {
        assert!(!should_prompt(&ctx), "{label} never prompts");
    }
    // The composition is today's unbound shape plus the notice.
    let plain = temp_dir("headless-unbound");
    let state = temp_dir("headless-unbound-state");
    let universe = SessionUniverse::compose(&session_runtime(), None, None, true, &plain, &state)
        .expect("composition succeeds without a root");
    assert!(universe.root().is_none(), "nothing binds headlessly");
    assert!(
        !universe.host_composed(),
        "no root, no lane — headlessly too"
    );
    assert!(
        universe
            .notice
            .as_deref()
            .is_some_and(|notice| notice.contains("No workspace is bound")),
        "G1's notice still says so: {:?}",
        universe.notice
    );
    let _ = (fs::remove_dir_all(&plain), fs::remove_dir_all(&state));
}
