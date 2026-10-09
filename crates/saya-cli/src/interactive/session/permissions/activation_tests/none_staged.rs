use super::super::{BYPASS_ON, HOST_LANE_FACT, NO_INTERPRETERS_STAGED, bypass_line};

/// With nothing staged, the line is the none-staged sentence instead
/// (DESIGN §5, verbatim): the mode said, and the door it does not open
/// named — never a silence.
#[test]
fn the_none_staged_variant_names_the_refusal_instead() {
    let line = bypass_line(&[], false, false, &[]);
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
    assert!(
        !line.contains(HOST_LANE_FACT),
        "the uncomposed line states no lane fact: {line}"
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
    let line = bypass_line(&[], false, false, &[]);
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
