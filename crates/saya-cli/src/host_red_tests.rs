//! H2 — the prompt, the grant, the per-call journal.
//!
//! Each test below names a seam H2 adds: the host prompt body, the name-only
//! grant, and the allow-side journal event. Written before any of them
//! exists; the failing output is pasted verbatim into `REPORT.md` before any
//! green edit.

use crate::approval_facts::{ApprovalFacts, HostFacts};
use crate::grant_token::{grant_family, grant_token};
use serde_json::json;
use std::path::PathBuf;

/// The fixed facts the host prompt states.
fn host_facts() -> ApprovalFacts {
    ApprovalFacts {
        host: Some(HostFacts {
            workspace_root: PathBuf::from("/home/user/proj"),
            timeout_seconds: 600,
            pass_env: Vec::new(),
        }),
        ..ApprovalFacts::default()
    }
}

/// The trap: a naive port of `run_program` facts would print a sandbox line.
/// The host lane states the absence in the same slot instead.
#[test]
fn the_host_prompt_never_prints_a_sandbox_line() {
    let body = crate::approval_facts::call_facts(
        "run_command",
        &json!({"program": "npm", "args": ["install"]}),
        Some("command:npm"),
        &host_facts(),
        None,
        None,
    )
    .expect("the host lane states its own facts");
    assert!(
        !body.contains("  sandbox:"),
        "the host lane is unsandboxed: no sandbox line, ever: {body}"
    );
    assert!(
        body.contains("  no sandbox:"),
        "the absence is said in the sandbox line's slot: {body}"
    );
}

/// The no-sandbox line's bytes are pinned.
#[test]
fn the_no_sandbox_line_names_user_network_filesystem() {
    let body = crate::approval_facts::call_facts(
        "run_command",
        &json!({"program": "npm", "args": ["install"]}),
        Some("command:npm"),
        &host_facts(),
        None,
        None,
    )
    .expect("the host lane states its own facts");
    for pin in [
        "no sandbox: runs as your user",
        "your whole filesystem",
        "your network",
        "unconfined",
    ] {
        assert!(
            body.contains(pin),
            "the no-sandbox line carries {pin:?}: {body}"
        );
    }
}

/// One name-only grant: any argv under the granted program pre-answers, and
/// any other program still asks.
#[test]
fn a_grant_pre_answers_any_argv_of_the_program_and_re_asks_any_other_program() {
    let facts = host_facts();
    let policy = saya_agent::SessionPolicy::new(saya_agent::ApprovalPolicy::Ask);
    assert!(
        policy.record(saya_agent::ApprovalChoice::AllowSession {
            token: "command:npm".to_owned(),
        }),
        "the first grant is new"
    );
    let tool = crate::interactive::session_definitions::run_command();
    for argv in [
        json!({"program": "npm", "args": ["install"]}),
        json!({"program": "npm", "args": ["run", "build"]}),
        json!({"program": "npm"}),
    ] {
        let token = grant_token(&tool.name, &argv, None, &facts);
        assert_eq!(
            token.as_deref(),
            Some("command:npm"),
            "any argv under the granted program suggests its token: {argv}"
        );
        assert_eq!(
            policy.resolve(&tool.effect, token.as_deref()),
            saya_agent::ApprovalDecision::Allow,
            "the held grant pre-answers any argv: {argv}"
        );
    }
    let other = json!({"program": "make", "args": ["check"]});
    assert_eq!(
        grant_token(&tool.name, &other, None, &facts),
        Some("command:make".to_owned()),
        "a different name suggests its own token"
    );
    let policy = saya_agent::SessionPolicy::new(saya_agent::ApprovalPolicy::Ask);
    assert_eq!(
        policy.resolve(&tool.effect, Some("command:make")),
        saya_agent::ApprovalDecision::Ask,
        "a different program still asks"
    );
}

/// A shell- or interpreter-family name adds the warning with this lane's
/// clause. The token stays `command:`, never an interpreter-family word.
#[test]
fn interpreter_family_names_add_the_warning_and_command_tokens_never_interpreter_ones() {
    let facts = host_facts();
    let tool = crate::interactive::session_definitions::run_command();
    for program in ["bash", "python3", "node"] {
        let arguments = json!({"program": program, "args": ["-c", "echo hi"]});
        let token = grant_token(&tool.name, &arguments, None, &facts);
        assert_eq!(
            token,
            Some(format!("command:{program}")),
            "no family mirror: command:{program} is the token"
        );
        let body = crate::approval_facts::call_facts(
            "run_command",
            &arguments,
            Some(format!("command:{program}").as_str()),
            &facts,
            None,
            None,
        )
        .expect("the host lane states its own facts");
        assert!(
            body.contains("the model writes the program")
                && body.contains("there is no sandbox")
                && body.contains("it runs as you"),
            "the interpreter warning rides the prompt with this lane's clause: {body}"
        );
    }
}

/// Every host call is journalled before the child spawns.
#[test]
fn every_host_call_is_journalled_before_the_child_spawns() {
    let dir = std::env::temp_dir().join(format!("saya-h2-journal-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("state dir creates");
    let journal = saya_store::SessionJournal::open(&dir);
    journal
        .command("npm", &["install".to_owned()], "run_command")
        .expect("the allow-side event journals");
    let events = journal.read().expect("the journal reads");
    assert_eq!(
        events,
        vec![saya_store::JournalEvent::Command {
            program: "npm".to_owned(),
            argv: vec!["install".to_owned()],
            door: "run_command".to_owned(),
        }],
        "shape: the program, its argv, and the door"
    );
    let raw = std::fs::read_to_string(dir.join("journal.ndjson")).expect("the journal reads");
    assert!(
        raw.contains("session-command") && !raw.contains("session-command-denied"),
        "the allow side writes its own kind: {raw}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Prefix-shaped grants refuse at parse.
#[test]
fn prefix_shaped_grants_refuse_at_parse() {
    let error = match crate::commands::run::scopes::parse(
        &["command:cargo test".to_owned()],
        crate::commands::run::scopes::Surface::Session,
    ) {
        Err(error) => error,
        Ok(approved) => panic!(
            "a prefix grant is a comforting fiction and must refuse: {:?}",
            approved.tokens
        ),
    };
    assert!(
        error.contains("bare name"),
        "the refusal states the shape rule: {error}"
    );
}

/// Once any host command has run, `run_program` gains the integrity line.
#[test]
fn run_program_s_prompt_gains_the_integrity_line_after_any_host_call() {
    let tool = crate::interactive::session_definitions::run_program(saya_agent::ToolDefinition {
        name: "run_program".into(),
        description: String::new(),
        read_only: false,
        parameters: serde_json::json!({"type": "object"}),
        effect: saya_agent::ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: saya_agent::LocalStateEffect::WriteWorkspace,
        },
        completion: None,
    });
    let arguments = json!({"program": "bench"});
    let mut facts = crate::approval_facts::ApprovalFacts {
        runner: Some(crate::approval_facts::RunnerFacts {
            fs_roots: vec![PathBuf::from("/home/user/proj")],
            runner_programs: vec!["bench".into()],
            ..crate::approval_facts::RunnerFacts::default()
        }),
        ..crate::approval_facts::ApprovalFacts::default()
    };
    let before =
        crate::prompt_approval::approval_prompt(&tool, &arguments, None, &facts, None, None);
    assert!(
        !before.contains("staged program integrity is outside saya's control"),
        "no host call yet: no integrity line: {before}"
    );
    facts.host_ran = true;
    let after =
        crate::prompt_approval::approval_prompt(&tool, &arguments, None, &facts, None, None);
    assert!(
        after.contains(
            "a host command ran in this session; staged program integrity is outside saya's control"
        ),
        "after any host call the integrity line rides: {after}"
    );
}

/// A `command:` token is never offered where the lane did not compose.
#[test]
fn a_command_token_is_never_offered_where_the_lane_did_not_compose() {
    let tool = crate::interactive::session_definitions::run_command();
    let arguments = json!({"program": "npm", "args": ["install"]});
    assert_eq!(
        grant_token(&tool.name, &arguments, None, &ApprovalFacts::default()),
        None,
        "no composed lane, no command: offer"
    );
    let token = grant_token(&tool.name, &arguments, None, &host_facts());
    assert_eq!(
        token.as_deref(),
        Some("command:npm"),
        "the composed lane offers the name-only token"
    );
    assert_eq!(
        grant_family("command:npm"),
        Some("command"),
        "the session-history line groups the family"
    );
}
