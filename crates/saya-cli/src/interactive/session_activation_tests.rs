//! The activation line's property tests (DESIGN test 9): the no-euphemism
//! line, the staged names, the session fork fact, and the none-staged
//! variant — every fact the surfaces must say, byte-pinned here.

use super::{BYPASS_ON, NO_INTERPRETERS_STAGED, bypass_line};

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

/// The none-staged sentence starts after the sentence break (U6 defect 3):
/// the mode fact ends its own line, and the design's sentence follows
/// verbatim on the next line — one line per fact, the join the staged
/// branch and the probe notice use. The old join was a space, which read
/// "applies. no interpreters" — a run-on with a lowercase word starting a
/// sentence mid-line. The sentence's bytes are the design's, so the break
/// is where the two facts meet, never inside a sentence.
#[test]
fn the_none_staged_sentence_starts_after_the_sentence_break() {
    let line = bypass_line(&[], false);
    let (mode_fact, rest) = line
        .split_once('\n')
        .expect("the none-staged sentence begins on its own line after the mode fact");
    assert_eq!(
        mode_fact, BYPASS_ON,
        "the mode fact is the first line, whole: {line}"
    );
    assert!(
        rest.starts_with(NO_INTERPRETERS_STAGED),
        "the design's sentence follows verbatim: {rest}"
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

/// The journal's bypass-activation decision (U7), as a property of the
/// system: the journal records consents, so it records exactly the moments
/// the user consented to the mode. A fresh session runs under the mode its
/// launch stated — bypass there is an activation. A resume whose
/// `--approval-mode` explicitly overrode the record is a new statement of
/// consent this process made. A resume that merely carries the persisted
/// mode re-prints the line but consents to nothing new — the record made
/// the mode operative — so nothing is journalled for it. An unparseable
/// mode is never bypass.
#[test]
fn the_launch_journals_bypass_exactly_when_this_process_consented_to_it() {
    use super::bypass_activated_at_launch;
    // A fresh session under the mode its launch stated.
    assert!(
        bypass_activated_at_launch(true, false, "bypass"),
        "a fresh session under bypass is an activation"
    );
    // A resume whose flag explicitly overrode the persisted mode.
    assert!(
        bypass_activated_at_launch(false, true, "bypass"),
        "an explicit `--approval-mode bypass` on a resume is a new consent"
    );
    // A resume that merely carries the persisted mode: the line re-prints,
    // but nothing new is consented to, so nothing is journalled.
    assert!(
        !bypass_activated_at_launch(false, false, "bypass"),
        "a carried bypass mode is the record's consent, not this process's"
    );
    // Any other mode journals nothing.
    for mode in ["ask", "read-only", "never", ""] {
        assert!(
            !bypass_activated_at_launch(true, false, mode),
            "no line for {mode:?}: only bypass activation is journalled"
        );
    }
}

/// The mid-session `/approvals bypass` journals a consent only when the
/// command newly activated the mode: the mode was not bypass before it and
/// is bypass after. A re-statement over an already-bypass session changes
/// nothing — like `/allow` over an already-granted token it says so, but
/// records no new consent — and a flip to a narrower mode is a narrowing,
/// never a widening, so it records none either.
#[test]
fn the_command_journals_bypass_only_when_it_newly_activates_the_mode() {
    use super::bypass_activated_by_command;
    // ask -> bypass: the consent the line announces.
    assert!(
        bypass_activated_by_command("ask", "bypass"),
        "the command that flips the mode is the activation it prints"
    );
    // bypass -> bypass: a re-statement. Nothing changed, nothing consented.
    assert!(
        !bypass_activated_by_command("bypass", "bypass"),
        "a re-statement over an already-bypass session records no new consent"
    );
    // bypass -> ask: a narrowing, not a widening.
    assert!(
        !bypass_activated_by_command("bypass", "ask"),
        "a narrowing mode change grants nothing"
    );
    // read-only -> bypass: still an activation — the widening is real.
    assert!(
        bypass_activated_by_command("read-only", "bypass"),
        "the widening is real whatever the mode it came from"
    );
}
