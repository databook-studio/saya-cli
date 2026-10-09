use super::*;

/// The Plan paragraph's bytes cannot drift unnoticed: pinned here and in
/// `assemble_system_prompt_for_mode`.
#[test]
fn plan_paragraph_bytes_are_pinned() {
    assert_eq!(
        PLAN_SYSTEM_PROMPT,
        "You are in Plan mode: investigate and answer with a plan. \
        Write-shaped tools are absent from your tool list and would refuse if called; \
        do not promise edits as if you had made them — describe the change you would make instead. \
        Plan composes with the approval policy and never widens it."
    );
}

/// Under Build the mode-aware prompt is byte-identical to the unmoded
/// session-aware prompt for the same session inputs — the append-only Plan
/// difference, asserted directly on the session-aware entry points.
#[test]
fn build_prompt_is_byte_identical_to_the_unmoded_prompt() {
    for (reg, mode, reachable) in [
        (single_registry("main"), MemoryMode::Off, false),
        (single_registry("main"), MemoryMode::Assisted, true),
        (single_registry("main"), MemoryMode::Assisted, false),
        (multi_registry(), MemoryMode::Off, false),
        (multi_registry(), MemoryMode::Assisted, true),
        (multi_registry(), MemoryMode::Off, true),
    ] {
        assert_eq!(
            assemble_system_prompt_for_mode(
                &reg,
                mode,
                reachable,
                &facts(&reg, &None),
                AgentMode::Build
            ),
            assemble_system_prompt_with_session(&reg, mode, reachable, &facts(&reg, &None)),
            "Build must be byte-identical to the unmoded prompt",
        );
    }
}

/// Under Plan the paragraph appears exactly once, and appending it is the
/// only difference from Build.
#[test]
fn plan_prompt_appends_the_paragraph_exactly_once_and_nothing_else() {
    for (reg, mode, reachable) in [
        (single_registry("main"), MemoryMode::Off, false),
        (single_registry("main"), MemoryMode::Assisted, true),
        (multi_registry(), MemoryMode::Off, true),
    ] {
        let build = assemble_system_prompt_for_mode(
            &reg,
            mode,
            reachable,
            &facts(&reg, &None),
            AgentMode::Build,
        )
        .expect("a prompt");
        let plan = assemble_system_prompt_for_mode(
            &reg,
            mode,
            reachable,
            &facts(&reg, &None),
            AgentMode::Plan,
        )
        .expect("a prompt");
        assert_eq!(
            plan.matches(PLAN_SYSTEM_PROMPT).count(),
            1,
            "the paragraph must appear exactly once: {plan}"
        );
        assert_eq!(
            plan,
            format!("{build}\n\n{PLAN_SYSTEM_PROMPT}"),
            "appending the paragraph must be the only difference",
        );
    }
}

/// Pin: every `ANSWER_CONTRACT` bullet is byte-identical to `release/0.4.1`.
/// Each clause fixes a measured class of benchmark failure; rewording risks
/// regressions for zero truth gain, so a future tidy-up must trip here first.
/// Only the header and the query-results scoping line may differ.
#[test]
fn answer_contract_bullets_are_byte_identical_to_release_0_4_1() {
    let bullets = [
        "- Return only the columns the question asks for; drop intermediate working columns.",
        "- Do not round unless asked.",
        "- Write dates as ISO YYYY-MM-DD.",
        "- If the question asks for a ratio, a percentage, or a difference, compute that value and answer with it — returning the operands alone stops one step short.",
        "- A superlative — \"the fastest\", \"the highest\", \"the top one\", \"the fewest\" — asks which one: answer with that row and the value that makes it so, not the ranking it came from. Every row tied with it is part of the answer.",
        "- Answer every quantity the question names; if it asks for two things, answer both.",
        "- Read measure words literally: \"volume\" is units, \"revenue\" is money.",
        "- A qualifier on a metric is not a qualifier on the population — filter the metric, not the rows.",
        "- Keep every row tied at a cut-off; never drop a tie to fit a limit.",
        "- When a period is named, enumerate that whole period, not only the rows that happen to appear in the data.",
    ];
    for bullet in bullets {
        assert!(
            ANSWER_CONTRACT.contains(bullet),
            "contract bullet changed or missing: {bullet}\n{ANSWER_CONTRACT}"
        );
    }
    assert_eq!(
        ANSWER_CONTRACT
            .lines()
            .filter(|line| line.starts_with("- "))
            .count(),
        10,
        "no bullet may be added or removed without tripping this pin: {ANSWER_CONTRACT}"
    );
}

/// The column-selection rules are nonsense for a turn whose answer is "I wrote
/// the file". The contract carries one scoping line limiting it to answers
/// that report query results; every other answer is untouched.
#[test]
fn answer_contract_scopes_itself_to_query_results() {
    assert!(
        ANSWER_CONTRACT.contains("govern answers that report query results"),
        "the contract must scope itself to query-result answers: {ANSWER_CONTRACT}"
    );
    assert!(
        ANSWER_CONTRACT.contains("leave other answers untouched"),
        "the contract must leave non-query answers alone: {ANSWER_CONTRACT}"
    );
}

/// A workspace-only session has no database, so the assembled prompt must not
/// assert "from this database" as the only place an answer can come from. The
/// stopping rule covers whatever the session is actually working with.
#[test]
fn working_guidance_does_not_scope_answers_to_the_database() {
    let prompt = assemble_system_prompt_with_session(
        &single_registry("main"),
        MemoryMode::Off,
        false,
        &facts(&single_registry("main"), &None),
    )
    .expect("a prompt");
    assert!(
        !prompt.contains("from this database"),
        "the stopping rule must cover the whole session, not just a database: {prompt}"
    );
}

/// The schema-discovery advice is genuinely database counsel and stays — as
/// the database case — in the assembled prompt of a database session.
#[test]
fn database_session_prompt_keeps_schema_discovery_advice() {
    let prompt = assemble_system_prompt_with_session(
        &single_registry("main"),
        MemoryMode::Off,
        false,
        &facts(&single_registry("main"), &None),
    )
    .expect("a prompt");
    assert!(
        prompt.contains("discover the schema before you query it"),
        "a database session must keep the schema-discovery advice: {prompt}"
    );
    assert!(
        prompt.contains("a missing table or column"),
        "a database session must keep the missing-table specifics: {prompt}"
    );
}

/// The paragraph says what Plan means and how it is enforced, without
/// overclaiming: it names Plan mode, the absent-then-refusing tools, the
/// no-promised-edits rule, and the composition with the approval policy —
/// and it never claims reads are unrestricted.
#[test]
fn plan_paragraph_says_the_mechanism_without_overclaiming_reads() {
    let reg = single_registry("main");
    let plan = assemble_system_prompt_for_mode(
        &reg,
        MemoryMode::Off,
        false,
        &facts(&reg, &None),
        AgentMode::Plan,
    )
    .expect("a prompt");
    assert!(
        plan.contains("You are in Plan mode: investigate and answer with a plan"),
        "must say the session is in Plan mode: {plan}"
    );
    assert!(
        plan.contains("absent from your tool list and would refuse if called"),
        "must state the honest mechanism — absent, and refusing if called: {plan}"
    );
    assert!(
        plan.contains("describe the change you would make instead"),
        "must tell it to describe the change, not promise made edits: {plan}"
    );
    assert!(
        plan.contains("Plan composes with the approval policy and never widens it"),
        "must state Plan composes with the policy: {plan}"
    );
    assert!(
        !plan.contains("unrestricted"),
        "must not claim reads are unrestricted: {plan}"
    );
}
