use super::*;

// ---------------------------------------------------------------------------
// 1. terminal_unbound_prompts_and_trust_binds_cwd
// ---------------------------------------------------------------------------

/// On a terminal, a fresh unbound session prompts at startup, and answering
/// trust binds exactly the launch cwd — like an explicit `--workspace` — so
/// the lane composes and the write-shaped tools appear.
#[test]
fn terminal_unbound_prompts_and_trust_binds_cwd() {
    assert!(
        should_prompt(&unbound_terminal()),
        "a fresh unbound terminal session prompts at startup"
    );
    // The prompt's register: the fact, the consequence, the exits.
    assert!(
        TRUST_PROMPT.contains("No workspace is bound"),
        "the prompt names the fact: {TRUST_PROMPT:?}"
    );
    assert!(
        TRUST_PROMPT.contains("file tools") && TRUST_PROMPT.contains("run_program"),
        "the prompt names the consequence: {TRUST_PROMPT:?}"
    );
    assert!(
        TRUST_PROMPT.contains("nothing is remembered"),
        "the prompt says trust is session-only: {TRUST_PROMPT:?}"
    );
    // Answering trust binds the cwd itself.
    let cwd = temp_dir("trust-cwd");
    let answer = ask_trust(&mut "t\n".as_bytes(), &mut Vec::new()).expect("the trust answer reads");
    assert!(
        matches!(answer, TrustAnswer::TrustCwd),
        "the `t` answer trusts this folder: {answer:?}"
    );
    let runtime = session_runtime();
    let state = temp_dir("trust-cwd-state");
    let universe = SessionUniverse::compose(&runtime, Some(&cwd), None, true, &cwd, &state)
        .expect("a trusted folder binds exactly like an explicit --workspace");
    assert_eq!(
        universe.root().expect("trust binds the cwd"),
        std::fs::canonicalize(&cwd).unwrap().as_path(),
        "trust binds exactly the launch cwd"
    );
    if cfg!(windows) {
        assert!(
            !universe.host_composed(),
            "Windows binds the trusted root while keeping its unavailable host lane off"
        );
    } else {
        assert!(
            universe.host_composed(),
            "with the root bound, the lane composes"
        );
    }
    let names: Vec<String> = universe
        .definitions(
            saya_agent::AgentMode::Build,
            ApprovalPolicy::Ask,
            true,
            true,
            false,
            false,
        )
        .into_iter()
        .map(|definition| definition.name)
        .collect();
    assert!(
        names.contains(&"workspace_write".to_string())
            && names.contains(&"workspace_edit".to_string()),
        "trust restores the file tools: {names:?}"
    );
    let _ = (fs::remove_dir_all(&cwd), fs::remove_dir_all(&state));
}

// ---------------------------------------------------------------------------
// extras the list misses, with why
// ---------------------------------------------------------------------------

/// Why: the design gives the prompt's register (fact, consequence, exits) —
/// a rewording that drops an exit is a regression no property above catches.
#[test]
fn the_trust_prompt_offers_all_three_exits() {
    for exit in ["[t]rust once", "[w]orkspace", "[c]ontinue unbound"] {
        assert!(
            TRUST_PROMPT.contains(exit),
            "the prompt offers every exit, missing {exit}: {TRUST_PROMPT:?}"
        );
    }
}

/// Why: an invalid answer must not bind, must not error the session — it
/// re-asks, and EOF (a vanishing stdin) continues unbound rather than
/// hanging or refusing startup.
#[test]
fn an_unparseable_answer_re_asks_and_eof_continues_unbound() {
    let answer =
        ask_trust(&mut "x\nc\n".as_bytes(), &mut Vec::new()).expect("a re-asked answer reads");
    assert!(
        matches!(answer, TrustAnswer::ContinueUnbound),
        "after a miss, a valid answer is read: {answer:?}"
    );
    let answer = ask_trust(&mut "".as_bytes(), &mut Vec::new()).expect("EOF reads");
    assert!(
        matches!(answer, TrustAnswer::ContinueUnbound),
        "EOF continues unbound, never hangs: {answer:?}"
    );
}

/// Why: the trusted-root line is the half of the activation pair the lane
/// fact does not carry — it must name the tree and the session-only shape.
#[test]
fn the_trusted_root_line_names_the_tree_and_the_session_only_shape() {
    let line = trusted_root_line(Path::new("/tmp/proj"));
    assert!(
        line.contains("/tmp/proj"),
        "the line names the just-trusted tree: {line}"
    );
    assert!(
        line.contains("this session only"),
        "the line states the session-only shape: {line}"
    );
}
