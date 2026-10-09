use super::super::{SESSION_FORK_FACT, bypass_line};

/// Under bypass with interpreters staged, the line names the staged names
/// inside the session wording of the run surface's warning: "bypass" (the
/// grammar's own word — no euphemism), the names, and the running
/// platform's fork fact. The run surface's "(where process-fork is
/// granted)" parenthetical is a run's clause and must not appear.
#[test]
fn bypass_activation_states_the_no_euphemism_line_naming_the_staged_interpreters() {
    let line = bypass_line(
        &["python3".to_string(), "perl".to_string()],
        false,
        false,
        &[],
    );
    assert!(
        line.contains("bypass on:"),
        "the line opens with the mode's own word, no euphemism: {line}"
    );
    assert!(
        line.contains("every tool call runs without asking"),
        "the line says what bypass does: {line}"
    );
    assert!(
        line.contains("every structural guard still applies"),
        "the line says what bypass does not touch: {line}"
    );
    assert!(
        line.contains("this session may execute python3, perl as an interpreter"),
        "the line names the staged interpreters: {line}"
    );
    // (Moved assertion, U8: the fork fact is the running platform's own —
    // the pin asserts the platform's clause appears, and
    // `the_fork_fact_says_only_what_the_running_platform_enforces` below
    // pins that clause's content per platform. The macOS bytes this
    // asserted are that platform's clause verbatim.)
    assert!(
        line.contains(SESSION_FORK_FACT),
        "the session fork fact is stated, not the run's conditional: {line}"
    );
    assert!(
        line.contains("What the interpreter computes is not a reviewed, fixed binary"),
        "the warning's honest close is carried: {line}"
    );
    assert!(
        !line.contains("where process-fork is granted"),
        "the run surface's parenthetical is a run's clause: {line}"
    );
    for euphemism in ["yolo", "danger mode", "auto", "unrestricted"] {
        assert!(
            !line.to_ascii_lowercase().contains(euphemism),
            "no friendlier word than bypass appears: {line}"
        );
    }
}
