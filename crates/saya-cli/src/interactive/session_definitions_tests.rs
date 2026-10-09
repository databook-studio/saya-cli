use super::*;
use saya_agent::ApprovalPolicy;
use saya_agent::SessionPolicy;

/// True when `text` states `phrase` as a standalone claim: the phrase
/// with no letter glued to either side. The honest "unsandboxed" must
/// never trip the "sandboxed" ban — "un" is a letter — while "runs
/// sandboxed" must, so the check is character-class matching, not
/// substring matching.
fn states_claim(text: &str, phrase: &str) -> bool {
    text.match_indices(phrase).any(|(start, _)| {
        let glued_before = text[..start]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphabetic());
        let glued_after = text[start + phrase.len()..]
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic());
        !glued_before && !glued_after
    })
}

/// The scratch database's lifetime, stated truthfully (U7): the old
/// description claimed the database "dies with" the session, which was
/// the one sentence in the toolset that was not true — nothing deletes
/// it, and a resumed session re-enters the same state directory and
/// re-opens the same file with its staged tables (pinned end-to-end by
/// `session_universe_tests::a_resumed_session_re_enters_its_state_dir_
/// and_reopens_the_scratch`). The description must state what persists
/// and never imply an end that does not exist.
#[test]
fn the_scratch_description_states_what_persists_and_never_claims_it_dies() {
    let description = scratch_sql().description;
    assert!(
        description.contains("state directory"),
        "the database's home is stated: {description}"
    );
    assert!(
        description.contains("persists across the process"),
        "the persistence is stated: {description}"
    );
    assert!(
        description.contains("resuming the session re-opens it"),
        "the resume behaviour is stated: {description}"
    );
    assert!(
        description.contains("nothing deletes it"),
        "what ends it is stated, if anything does: {description}"
    );
    assert!(
        !description.contains("dies with"),
        "the false claim must not come back in any wording: {description}"
    );
}

/// The session's `run_command` is ask-gated: the engine decides every
/// call, and the description steers the model to the contained lane when
/// a program is allowlisted there — while never implying the lane is
/// contained.
#[test]
fn the_session_s_run_command_is_ask_gated_and_honest() {
    let definition = run_command();
    assert_eq!(definition.name, "run_command");
    assert!(
        definition.effect.requires_approval,
        "the session's run_command is ask-gated: the engine decides every call"
    );
    assert!(
        definition.effect.external_side_effect,
        "unsandboxed network and filesystem are an external side effect"
    );
    let description = &definition.description;
    assert!(
        description.contains("unsandboxed"),
        "the posture is said in the sandbox line's slot: {description}"
    );
    assert!(
        description.contains("prefer run_program"),
        "the description steers to the contained lane: {description}"
    );
    // "contained" appears only in the steering clause ("the contained
    // lane is strictly safer" — the *other* lane's property, naming
    // where to go instead); the ban is on implying *this* lane contains.
    // Matched as a standalone claim, not a substring: the honest
    // "unsandboxed" carries "sandboxed" inside it, and a substring ban
    // would fail on the truthful posture it exists to protect.
    for contained in [
        "sandboxed",
        "contained execution",
        "bounded to",
        "no network",
        "egress: none",
    ] {
        assert!(
            !states_claim(description, contained),
            "never imply containment ({contained}): {description}"
        );
    }
    // The check still bites: the honest word reworded into a genuine
    // containment claim must trip it, so the matcher above cannot pass
    // vacuously on a weakened predicate. Every other banned phrase
    // trips on the description with its honest guards stripped, in the
    // same test that asserts the honest description passes.
    let claiming = description.replace("unsandboxed", "runs sandboxed");
    assert!(
        states_claim(&claiming, "sandboxed"),
        "a genuine containment claim must trip the check: {claiming}"
    );
    for (honest, claiming, phrase) in [
        (
            "prefer run_program — the contained lane is strictly safer",
            "prefer run_program — contained execution for this lane",
            "contained execution",
        ),
        (
            "what the program spawns, downloads, or executes is not bounded",
            "what the program spawns is bounded to the workspace",
            "bounded to",
        ),
        (
            "as your user, with your whole filesystem and your network, unconfined",
            "with your whole filesystem and no network",
            "no network",
        ),
        (
            "with your whole filesystem and your network, unconfined",
            "egress: none for this lane",
            "egress: none",
        ),
    ] {
        assert!(
            description.contains(honest),
            "the honest guard the case rewrites must be present: {description}"
        );
        let rewritten = description.replace(honest, claiming);
        assert!(
            states_claim(&rewritten, phrase),
            "a genuine {phrase:?} claim must trip the check: {rewritten}"
        );
    }
    assert!(
        !description.contains("command:*") && !description.contains("prefix"),
        "no blanket or prefix fiction in the description: {description}"
    );
}

/// The session's `run_program` is ask-gated, not plan-gated: the run
/// surface's definition is plan-gated (`requires_approval: false`, the
/// plan approval answering for every call), but the session has no plan —
/// the ask is the per-call consent, so the definition this module
/// advertises must carry `requires_approval: true`. A definition that
/// spread the run surface's effect would auto-run any allowlisted program
/// with no ask at all — advertised-but-unasked, the anti-pattern this
/// module exists to prevent — while its own doc promises the ask.
#[test]
fn the_session_s_run_program_is_ask_gated_not_plan_gated() {
    // The run surface's own shape: plan-gated, workspace-writing, no
    // external side effect (the session spawn declares no egress).
    let source = ToolDefinition {
        name: "run_program".into(),
        description: "the run surface's wording".into(),
        read_only: false,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::WriteWorkspace,
        },
        concurrency: ToolConcurrency::Serial,
        completion: Some("program ran".into()),
    };
    let definition = run_program(source);
    assert!(
        definition.effect.requires_approval,
        "the session's run_program is ask-gated: the engine decides every call"
    );
    assert!(
        !definition.effect.external_side_effect,
        "the empty egress stays empty — the rewrite adds no capability"
    );
    assert_eq!(
        definition.effect.local_state,
        LocalStateEffect::WriteWorkspace,
        "the write shape is unchanged"
    );
    // And so the engine asks under the session's mode, where the grant
    // token (`runner:<program>` / `interpreter:<program>`) is offered.
    let policy = SessionPolicy::new(ApprovalPolicy::Ask);
    assert_eq!(
        policy.resolve(&definition.effect, None),
        saya_agent::ApprovalDecision::Ask,
        "an ask session renders the ask for run_program"
    );
}
