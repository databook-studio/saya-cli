//! G3 (slices 6+7) — the startup trust prompt and the bypass × trust
//! intersection, written before any of it exists. Every test below names an
//! API that does not exist when written (`session_trust::*`); the red
//! compile failure is pasted verbatim into `REPORT.md`.

use std::fs;
use std::path::{Path, PathBuf};

use saya_agent::ApprovalPolicy;
use saya_config::{
    AiProvider, ColorChoice, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs, ResolvedJobs, ResolvedMemory,
    ResolvedRunnerJobs, ThemeChoice,
};

use super::session_trust::{
    BYPASS_UNBOUND_NO_LANE, TRUST_PROMPT, TrustAnswer, TrustPromptContext, ask_trust,
    bypass_no_lane_note, parse_trust_answer, resolve_trusted_dir, should_prompt, trusted_root_line,
};
use super::session_universe::SessionUniverse;

// ---------------------------------------------------------------------------
// harness
// ---------------------------------------------------------------------------

fn temp_dir(label: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("saya-session-trust-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn session_runtime() -> crate::config::runtime::RuntimeConfig {
    crate::config::runtime::RuntimeConfig {
        resolved: ResolvedConfig {
            profile_name: None,
            profile: None,
            ai: ResolvedAi {
                provider: AiProvider::Ollama,
                model: "test-model".into(),
                base_url: None,
                api_key: None,
                allow_data_sharing: true,
                temperature: 0.0,
                timeout_seconds: 60,
                idle_timeout_seconds: 90,
                max_output_tokens: 4096,
                max_output_tokens_is_default: true,
                context_byte_budget: 256 * 1024,
                context_window_tokens: None,
                show_thinking: false,
                retry_delays_ms: vec![250, 500, 1000],
            },
            max_rows: 100,
            read_only: true,
            max_iterations: 4,
            candidates: 1,
            jobs: ResolvedJobs {
                wall_clock_seconds: None,
                tokens_per_endpoint: Default::default(),
                turns: Some(4),
                tool_calls: None,
                fetch: ResolvedFetchJobs::default(),
                interpreter: ResolvedInterpreterJobs::default(),
                runner: ResolvedRunnerJobs::default(),
            },
            query_timeout_seconds: 5,
            output_format: OutputFormat::Text,
            output_color: ColorChoice::Auto,
            ui_theme: ThemeChoice::Auto,
            memory: ResolvedMemory {
                mode: MemoryMode::Off,
                max_contracts: 5,
                max_claims_per_contract: 12,
                max_context_bytes: 16384,
            },
            host_commands: ResolvedHostCommands::default(),
            session_deny: Default::default(),
            ignored_project_overrides: Vec::new(),
            endpoints: Default::default(),
        },
        connections: Default::default(),
        config_path: None,
        connections_path: None,
        cache_scope: PathBuf::from("/tmp/saya-session-trust"),
        secret_values: Default::default(),
    }
}

/// A fresh unbound launch: no `--workspace`, no pin, outside any worktree.
fn unbound_terminal() -> TrustPromptContext {
    TrustPromptContext {
        is_terminal: true,
        fresh: true,
        has_explicit: false,
        has_pin: false,
        root_bound: false,
        turn_file: false,
    }
}

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
    assert!(
        universe.host_composed(),
        "with the root bound, the lane composes"
    );
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
        names.contains(&"workspace_write".to_string()),
        "trust restores the file tools: {names:?}"
    );
    let _ = (fs::remove_dir_all(&cwd), fs::remove_dir_all(&state));
}

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
        !names.contains(&"workspace_write".to_string()),
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
    assert!(universe.host_composed(), "the lane composes after trust");
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
        names.contains(&"run_command".to_string()),
        "after trust, the lane runs unasked under bypass: {names:?}"
    );
    // The activation line names both exposures.
    let line = crate::interactive::session_activation::bypass_line(
        &[],
        false,
        universe.host_composed(),
        &[],
    );
    assert!(
        line.contains(crate::interactive::session_activation::HOST_LANE_FACT),
        "the activation line names the unsandboxed exposure: {line}"
    );
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
            let denied = crate::interactive::session_deny::denied_call_program(
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
    let open = crate::interactive::session_deny::denied_call_program(
        "run_command",
        &serde_json::json!({"program": "make"}),
        &["curl".to_owned()],
    );
    assert!(open.is_none(), "a name outside the list still runs");
}

// ---------------------------------------------------------------------------
// 8. nothing_about_trust_is_persisted
// ---------------------------------------------------------------------------

/// Trust is session-only: binding the trusted folder writes no dotfile, no
/// marker, no config key anywhere. The session record carries only the root
/// pin — today's shape — and re-trust is per-process.
#[test]
fn nothing_about_trust_is_persisted() {
    let cwd = temp_dir("trust-no-store");
    let before: Vec<String> = fs::read_dir(&cwd)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    let resolved = resolve_trusted_dir(&cwd).expect("the trusted dir resolves");
    let runtime = session_runtime();
    let state = temp_dir("trust-no-store-state");
    let universe = SessionUniverse::compose(&runtime, Some(&resolved), None, true, &cwd, &state)
        .expect("trust binds");
    assert!(universe.root().is_some(), "trust binds");
    let after: Vec<String> = fs::read_dir(&cwd)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        before, after,
        "trust writes no dotfile, no marker into the trusted folder"
    );
    assert!(
        !cwd.join(".saya-trust").exists() && !cwd.join(".saya").exists(),
        "no trust store appears in the folder"
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
