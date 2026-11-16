use super::*;

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

/// The session's `run_command` is ask-gated and honest: the engine decides
/// every call, the unsandboxed posture is stated, and nothing implies this
/// lane contains. The session composes no runner, so no clause names one.
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
    // The ban is on implying *this* lane contains. Matched as a standalone
    // claim, not a substring: the honest "unsandboxed" carries "sandboxed"
    // inside it, and a substring ban would fail on the truthful posture it
    // exists to protect.
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
