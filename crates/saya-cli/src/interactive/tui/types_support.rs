use super::super::SessionUsage;
use saya_agent::TokenUsage;

/// Helper: build a `TokenUsage` with only the two base counters set.
pub(crate) fn usage(input: u64, output: u64) -> TokenUsage {
    TokenUsage::new(input, output)
}

/// Learning-call usage: the separately-labelled learning section,
/// absent-vs-zero distinction, timeout safety, and learning-off output.
/// Moved byte-identical from the hub; no snapshots involved.
/// The extraction call's usage is labelled apart from the answering total. A
/// session that ran extraction reports a separate "Learning call" section; the
/// two totals must not be merged — the learning call is invisible in the
/// transcript, so nothing else would reveal the omission.
#[test]
fn extraction_usage_is_labelled_apart_from_the_answering_total() {
    let mut session = SessionUsage::default();
    session.record(&usage(300, 130));
    session.record_learning(Some(usage(50, 20)));
    let rendered = session.render();
    assert!(
        rendered.contains("Session token usage"),
        "answering section must be labelled, got:\n{rendered}"
    );
    assert!(
        rendered.contains("Input tokens: 300"),
        "answering input total must be present, got:\n{rendered}"
    );
    assert!(
        rendered.contains("Learning call"),
        "learning call must be labelled apart, got:\n{rendered}"
    );
    assert!(
        rendered.contains("Input tokens: 50"),
        "learning input total must be present and distinct, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("Input tokens: 350"),
        "learning and answering must not be merged into one total, got:\n{rendered}"
    );
}

/// A provider that reported nothing for extraction adds nothing and stays
/// distinguishable from one that reported zeros. `None` omits the learning
/// section entirely (absent is not zero); a reported zero shows the section
/// with zeros — the two must not collapse to the same display.
#[test]
fn extraction_that_reported_nothing_is_distinguishable_from_a_reported_zero() {
    let mut absent = SessionUsage::default();
    absent.record(&usage(100, 50));
    absent.record_learning(None);
    let absent_rendered = absent.render();

    let mut zero = SessionUsage::default();
    zero.record(&usage(100, 50));
    zero.record_learning(Some(usage(0, 0)));
    let zero_rendered = zero.render();

    assert!(
        !absent_rendered.contains("Learning call"),
        "absent extraction must not show a learning section, got:\n{absent_rendered}"
    );
    assert!(
        zero_rendered.contains("Learning call"),
        "a reported zero must show the learning section, got:\n{zero_rendered}"
    );
    assert_ne!(
        absent_rendered, zero_rendered,
        "absent and reported-zero must be distinguishable"
    );
    // The answering total is untouched in both.
    assert!(
        absent_rendered.contains("Input tokens: 100"),
        "answering total must be intact when extraction reported nothing, got:\n{absent_rendered}"
    );
    assert!(
        zero_rendered.contains("Input tokens: 100"),
        "answering total must be intact when extraction reported zero, got:\n{zero_rendered}"
    );
}

/// A timed-out extraction produced no response, so it reports no usage and
/// must not corrupt the totals: the answering total stays as recorded and the
/// learning section stays absent (no response means no number to report).
#[test]
fn a_timed_out_extraction_does_not_corrupt_the_totals() {
    let mut session = SessionUsage::default();
    session.record(&usage(300, 130));
    // A timeout yields no response, so the recorder is handed nothing.
    session.record_learning(None);
    let rendered = session.render();
    assert!(
        rendered.contains("Input tokens: 300"),
        "answering total must be intact, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("Learning call"),
        "a timeout must not invent a learning section, got:\n{rendered}"
    );
}

/// With learning disabled no extraction call runs, so `/usage` must render
/// byte-for-byte what it rendered before learning was counted — no learning section
/// appears and the answering breakdown is untouched.
#[test]
fn with_learning_off_usage_output_is_unchanged() {
    let mut session = SessionUsage::default();
    session.record(&TokenUsage::new(100, 50).with_cached_input(Some(80)));
    let rendered = session.render();
    let expected = "Session token usage (1 turn):\n\n\
     \x20 Input tokens: 100\n\
     \x20 Output tokens: 50\n\
     \x20 Reasoning tokens: —\n\
     \x20 Cached input: 80\n\
     \x20 Cache creation: —\n\
     \x20 Cache hit rate: 80% (Σcached / Σinput)";
    assert_eq!(
        rendered, expected,
        "answering-only render must be byte-identical with learning off"
    );
}
