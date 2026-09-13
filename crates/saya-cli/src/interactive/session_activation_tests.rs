//! The activation line's property tests (DESIGN test 9): the no-euphemism
//! line, the staged names, the session fork fact, and the none-staged
//! variant — every fact the surfaces must say, byte-pinned here.

use super::{NO_INTERPRETERS_STAGED, bypass_line};

/// Under bypass with interpreters staged, the line names the staged names
/// inside the session wording of the run surface's warning: "bypass" (the
/// grammar's own word — no euphemism), the names, and the measured fork
/// fact. The run surface's "(where process-fork is granted)" parenthetical
/// is false for sessions and must not appear.
#[test]
fn bypass_activation_states_the_no_euphemism_line_naming_the_staged_interpreters() {
    let line = bypass_line(&["python3".to_string(), "perl".to_string()], false);
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
    assert!(
        line.contains(
            "no process-fork is granted: children an interpreter spawns are refused by \
             the sandbox"
        ),
        "the session fork fact is stated, not the run's conditional: {line}"
    );
    assert!(
        line.contains("What the interpreter computes is not a reviewed, fixed binary"),
        "the warning's honest close is carried: {line}"
    );
    assert!(
        !line.contains("where process-fork is granted"),
        "the run surface's parenthetical is false for sessions: {line}"
    );
    for euphemism in ["yolo", "danger mode", "auto", "unrestricted"] {
        assert!(
            !line.to_ascii_lowercase().contains(euphemism),
            "no friendlier word than bypass appears: {line}"
        );
    }
}

/// With nothing staged, the line is the none-staged sentence instead
/// (DESIGN §5, verbatim): the mode said, and the door it does not open
/// named — never a silence.
#[test]
fn the_none_staged_variant_names_the_refusal_instead() {
    let line = bypass_line(&[], false);
    assert!(
        line.contains("bypass on:") && line.contains(NO_INTERPRETERS_STAGED),
        "the none-staged line names the door that stays shut: {line}"
    );
    assert!(
        line.contains("[jobs.interpreter] allow"),
        "the line names where to stage interpreters: {line}"
    );
    // No warning sentence without staged interpreters: nothing is being
    // granted, so nothing is warned about.
    assert!(
        !line.contains("interpreter approval"),
        "the staged-interpreter warning must not appear when none are staged: {line}"
    );
}

/// Where the probe refused, the activation line carries the same fact the
/// notice seam says: "everything runs" with an absent runner is a claim the
/// session must qualify, at the moment it says bypass on.
#[test]
fn the_activation_line_references_the_probe_refusal() {
    let line = bypass_line(&["python3".to_string()], true);
    assert!(
        line.contains("run_program is unavailable: the sandbox probe did not prove this host"),
        "the probe-refused fact is carried on the activation line: {line}"
    );
    let proven = bypass_line(&["python3".to_string()], false);
    assert!(
        !proven.contains("run_program is unavailable"),
        "a proven host says nothing about the probe: {proven}"
    );
}
