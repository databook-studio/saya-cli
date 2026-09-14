//! H3 red — bypass, status, docs: the lane's last slice.
//!
//! Each test below names a seam H3 adds: the lane fact on the bypass
//! activation line, the denied-names segment, the status header's `host:`
//! segment, and the help-parity extension. Written before any of them
//! exists; the failing output is pasted verbatim into `REPORT.md` before
//! any green edit.

use crate::interactive::session_activation::{HOST_LANE_FACT, bypass_line, denied_segment};

/// The bypass activation line states the lane when composed: host commands
/// run unsandboxed — as the user, their network, their filesystem.
#[test]
fn the_bypass_activation_line_states_the_lane_when_composed() {
    let line = bypass_line(&[], false, true, &[]);
    assert!(
        line.contains(HOST_LANE_FACT),
        "the composed line states the lane fact: {line}"
    );
}

/// The uncomposed activation line keeps today's exact bytes: adding the lane
/// fact must not silently reword the common case.
#[test]
fn the_uncomposed_activation_line_keeps_its_exact_bytes() {
    let line = bypass_line(&[], false, false, &[]);
    assert!(
        !line.contains(HOST_LANE_FACT),
        "the uncomposed line states no lane fact: {line}"
    );
    let staged = bypass_line(&["python3".to_owned()], false, false, &[]);
    assert!(
        !staged.contains(HOST_LANE_FACT),
        "the staged-interpreter variant states no lane fact either: {staged}"
    );
}

/// The activation line states the denied names when non-empty; an empty list
/// keeps today's bytes.
#[test]
fn the_activation_line_states_the_denied_names_when_nonempty() {
    let denied = denied_segment(&["curl".to_owned(), "ssh".to_owned()]);
    assert!(
        denied.contains("curl") && denied.contains("ssh"),
        "the denied segment names the list: {denied}"
    );
    assert!(
        denied_segment(&[]).is_empty(),
        "an empty list keeps today's bytes: no segment"
    );
}

/// Bypass allows every host ask and structural refusals still refuse:
/// lane-off, path-shaped, not-on-PATH, timeout, and denied.
#[test]
fn bypass_allows_every_host_ask_and_structural_refusals_still_refuse() {
    use saya_agent::{ApprovalDecision, ApprovalPolicy, SessionPolicy};
    // Bypass allows every host ask: no token, no grant, still Allow.
    let tool = crate::interactive::session_definitions::run_command();
    let policy = SessionPolicy::new(ApprovalPolicy::Bypass);
    assert_eq!(
        policy.resolve(&tool.effect, None),
        ApprovalDecision::Allow,
        "bypass allows every host ask without a grant"
    );
    // Path-shaped names refuse at the executor's own shape rule.
    assert!(
        saya_harness::host::HostCommand::new("bin/npm", Vec::<String>::new()).is_err(),
        "a path-shaped name refuses"
    );
    // A name not on the passed PATH refuses naming the PATH searched.
    let config = saya_harness::host::HostConfig::new(
        "/nonexistent-path-for-h3-red",
        std::path::PathBuf::from("/tmp"),
        std::time::Duration::from_secs(600),
    )
    .expect("the config builds");
    assert!(
        config.resolve("npm").is_err(),
        "a name not on the passed PATH refuses"
    );
    // Timeout 0 and over-ceiling refuse.
    assert!(config.narrow_timeout(0).is_err(), "timeout 0 refuses");
    assert!(
        config.narrow_timeout(3600).is_err(),
        "a timeout over the ceiling refuses"
    );
    // A denied name refuses before bypass's auto-allow (the decider-level
    // deny refusal carrying its typed wording to the model).
    let denied = crate::interactive::session_deny::denied_call_program(
        "run_command",
        &serde_json::json!({"program": "curl"}),
        &["curl".to_owned()],
    );
    assert_eq!(
        denied.as_deref(),
        Some("curl"),
        "a denied name refuses under bypass"
    );
    // Lane-off refuses the composition gate: the `/allow` composition
    // refusal names the unstated lane.
    let refusal = crate::interactive::allow_refusal::composition_refusal(
        "command:npm",
        &crate::approval_facts::ApprovalFacts::default(),
    );
    assert!(
        refusal
            .as_deref()
            .unwrap_or_default()
            .contains("--host-commands"),
        "lane-off refuses the host ask: {refusal:?}"
    );
}

/// The status header gains a `host:` segment and lists denied names.
#[test]
fn the_status_header_carries_the_host_segment_and_the_denied_names() {
    let mut state = crate::SessionState::new("s1", Some(String::from("analytics")), "qwen");
    state.host_composed = true;
    state.denied_programs = vec!["curl".to_owned()];
    let line = crate::interactive::session_prompt::status_line(&state);
    assert!(
        line.contains("host:unsandboxed"),
        "the status header gains the host: segment: {line}"
    );
    assert!(
        line.contains("curl"),
        "the status header lists denied names: {line}"
    );
}

/// The help-parity suite extends to the new token and both flags: the root
/// help names `--host-commands` and `--deny`, the `/allow` help names
/// `command:<program>`, and the `/allow` help states the deny list's
/// not-bounded clause.
#[test]
fn the_help_parity_suite_extends_to_the_token_and_both_flags() {
    use clap::CommandFactory as _;
    let root_help = crate::Cli::command().render_help().to_string();
    assert!(
        root_help.contains("--host-commands"),
        "the root help names the lane flag: {root_help}"
    );
    assert!(
        root_help.contains("--deny"),
        "the root help names the deny flag: {root_help}"
    );
    let allow_help = crate::slash::command_help("allow").expect("/allow has per-command help");
    assert!(
        allow_help.contains("command:<program>"),
        "the /allow help names the new token: {allow_help}"
    );
    assert!(
        allow_help.contains("allowed programs may still invoke it"),
        "the /allow help states the deny list's not-bounded clause: {allow_help}"
    );
}
