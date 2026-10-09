use super::*;

// ---------------------------------------------------------------------------
// 4. continue_unbound_keeps_today_s_shape
// ---------------------------------------------------------------------------

/// Answering `c` keeps today's exact behaviour: tools hidden, reads
/// refused, `ws:unbound` in the header. And a resume never prompts at all —
/// `--continue` on an already-pinned session re-opens the pin untouched.
#[test]
fn continue_unbound_keeps_today_s_shape() {
    let answer = parse_trust_answer("c").expect("the continue answer parses");
    assert!(
        matches!(answer, TrustAnswer::ContinueUnbound),
        "the `c` answer continues unbound: {answer:?}"
    );
    let plain = temp_dir("continue-unbound");
    let state = temp_dir("continue-unbound-state");
    let universe = SessionUniverse::compose(&session_runtime(), None, None, true, &plain, &state)
        .expect("composition succeeds without a root");
    assert!(universe.root().is_none(), "continue binds nothing");
    assert!(!universe.host_composed(), "continue composes no lane");
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
        !names.contains(&"workspace_write".to_string())
            && !names.contains(&"workspace_edit".to_string()),
        "the write tools stay hidden: {names:?}"
    );
    // A resume never prompts — pinned or pre-workspace alike.
    for (label, ctx) in [
        (
            "a pinned resume",
            TrustPromptContext {
                fresh: false,
                has_pin: true,
                ..unbound_terminal()
            },
        ),
        (
            "an unbound --continue",
            TrustPromptContext {
                fresh: false,
                ..unbound_terminal()
            },
        ),
        (
            "an explicit --workspace",
            TrustPromptContext {
                has_explicit: true,
                root_bound: true,
                ..unbound_terminal()
            },
        ),
    ] {
        assert!(!should_prompt(&ctx), "{label} never prompts");
    }
    let _ = (fs::remove_dir_all(&plain), fs::remove_dir_all(&state));
}

// ---------------------------------------------------------------------------
// 5. bypass_unbound_terminal_prompts_for_the_folder_then_runs_the_lane
// ---------------------------------------------------------------------------

/// Bypass is call consent, not session consent: under bypass + unbound +
/// terminal the folder prompt still appears first. After trust the lane
/// composes and calls run unasked — and the activation line names both
/// exposures: unsandboxed, and pointed at this tree.
#[test]
fn bypass_unbound_terminal_prompts_for_the_folder_then_runs_the_lane() {
    assert!(
        should_prompt(&unbound_terminal()),
        "bypass never silences the folder prompt on a terminal"
    );
    let cwd = temp_dir("bypass-trust");
    let runtime = session_runtime();
    let state = temp_dir("bypass-trust-state");
    let universe = SessionUniverse::compose(&runtime, Some(&cwd), None, true, &cwd, &state)
        .expect("trust binds under bypass exactly as under ask");
    if cfg!(windows) {
        assert!(
            !universe.host_composed(),
            "Windows trusts the folder without composing an unsupported host lane"
        );
    } else {
        assert!(universe.host_composed(), "the lane composes after trust");
    }
    let names: Vec<String> = universe
        .definitions(
            saya_agent::AgentMode::Build,
            ApprovalPolicy::Bypass,
            false,
            true,
            false,
            false,
        )
        .into_iter()
        .map(|definition| definition.name)
        .collect();
    if cfg!(windows) {
        assert!(
            !names.contains(&"run_command".to_string()),
            "bypass does not advertise Windows' unavailable host lane: {names:?}"
        );
    } else {
        assert!(
            names.contains(&"run_command".to_string()),
            "after trust, the lane runs unasked under bypass: {names:?}"
        );
    }
    // The activation line names both exposures.
    let line = crate::interactive::session::permissions::activation::bypass_line(
        &[],
        false,
        universe.host_composed(),
        &[],
    );
    if cfg!(windows) {
        assert!(
            !line.contains(crate::interactive::session::permissions::activation::HOST_LANE_FACT),
            "an inactive Windows lane contributes no unsandboxed exposure: {line}"
        );
    } else {
        assert!(
            line.contains(crate::interactive::session::permissions::activation::HOST_LANE_FACT),
            "the activation line names the unsandboxed exposure: {line}"
        );
    }
    let trusted = trusted_root_line(universe.root().expect("trust binds"));
    assert!(
        trusted.contains(&cwd.canonicalize().unwrap().display().to_string()),
        "the trust echo names the just-trusted tree: {trusted}"
    );
    let _ = (fs::remove_dir_all(&cwd), fs::remove_dir_all(&state));
}

// ---------------------------------------------------------------------------
// 6. bypass_unbound_headless_composes_no_lane_and_says_so
// ---------------------------------------------------------------------------

/// Bypass + unbound + non-terminal: no prompt is possible, so nothing binds
/// and the lane cannot compose either. Bypass runs with the lane absent and
/// says so — the fail-closed intersection.
#[test]
fn bypass_unbound_headless_composes_no_lane_and_says_so() {
    let ctx = TrustPromptContext {
        is_terminal: false,
        ..unbound_terminal()
    };
    assert!(!should_prompt(&ctx), "headless bypass never prompts");
    let plain = temp_dir("bypass-headless");
    let state = temp_dir("bypass-headless-state");
    let universe = SessionUniverse::compose(&session_runtime(), None, None, true, &plain, &state)
        .expect("composition succeeds without a root");
    assert!(
        !universe.host_composed(),
        "no root, no lane — bypass cannot bind silently"
    );
    let names: Vec<String> = universe
        .definitions(
            saya_agent::AgentMode::Build,
            ApprovalPolicy::Bypass,
            false,
            true,
            false,
            false,
        )
        .into_iter()
        .map(|definition| definition.name)
        .collect();
    assert!(
        !names.contains(&"run_command".to_string()),
        "bypass runs with the lane absent: {names:?}"
    );
    let note =
        bypass_no_lane_note(true, universe.host_composed()).expect("the absent lane is said");
    assert!(
        note.contains("run_command is unavailable"),
        "the note names the consequence: {note}"
    );
    assert!(
        note == BYPASS_UNBOUND_NO_LANE,
        "the note is the pinned bytes: {note:?}"
    );
    let _ = (fs::remove_dir_all(&plain), fs::remove_dir_all(&state));
}

// ---------------------------------------------------------------------------
// 7. a_denied_name_refuses_in_all_four_corners
// ---------------------------------------------------------------------------

/// Deny holds in all four corners: ask/bypass × trusted/unbound. A denied
/// name refuses under ask, under bypass, trusted folder or not.
#[test]
fn a_denied_name_refuses_in_all_four_corners() {
    for mode in [ApprovalPolicy::Ask, ApprovalPolicy::Bypass] {
        for trusted in [false, true] {
            let denied = crate::interactive::session::permissions::deny::denied_call_program(
                "run_command",
                &serde_json::json!({"program": "curl"}),
                &["curl".to_owned()],
            );
            assert_eq!(
                denied.as_deref(),
                Some("curl"),
                "a denied name refuses under {mode:?}, trusted={trusted}"
            );
        }
    }
    let open = crate::interactive::session::permissions::deny::denied_call_program(
        "run_command",
        &serde_json::json!({"program": "make"}),
        &["curl".to_owned()],
    );
    assert!(open.is_none(), "a name outside the list still runs");
}
