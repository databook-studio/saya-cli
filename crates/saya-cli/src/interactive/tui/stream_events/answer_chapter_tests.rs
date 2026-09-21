//! Chapter-boundary tests for the answer lifecycle: a reset or re-stream is
//! scoped to the chapter in flight, so an earlier chapter's delivered answer
//! is never erased by a retry. The audit sequence (F02) is replayed against
//! the transcript directly — no provider involved.

use super::answer::RETRY_NOTICE;
use super::{BlockKind, Transcript, apply_event};
use saya_agent::AgentEvent;

/// A comparable snapshot of the transcript: kind, chapter, and verbatim text
/// of every block, for whole-transcript equality without touching `Block`.
fn snapshot(transcript: &Transcript) -> Vec<(BlockKind, u32, String)> {
    transcript
        .blocks()
        .iter()
        .map(|b| (b.kind, b.chapter, b.text.clone()))
        .collect()
}

/// The audit's exact four-step sequence: a completed answer from an earlier
/// chapter is evidence the user already received, so a retry that begins a
/// new chapter — even one whose stream died before any text — must leave it
/// untouched, and the retried text must land in the new chapter.
#[test]
fn a_retry_in_a_new_chapter_leaves_the_previous_answer_alone() {
    let mut t = Transcript::new();
    // Chapter 1: question asked, answer delivered.
    t.push(BlockKind::User, "old question");
    apply_event(&mut t, AgentEvent::assistant_text("old answer"), false);
    // Chapter 2 opens; the stream fails before emitting any answer text, so
    // this chapter holds no assistant block at all.
    t.push(BlockKind::User, "new question");
    apply_event(&mut t, AgentEvent::turn_reset(), false);
    let old = t
        .blocks()
        .iter()
        .find(|b| b.kind == BlockKind::Assistant)
        .expect("the completed answer must survive the cross-chapter reset");
    assert_eq!(
        old.text,
        "old answer",
        "a completed answer from an earlier chapter must not be erased: {:?}",
        t.blocks()
    );
    assert_eq!(
        old.chapter,
        1,
        "the old answer must still sit in its own chapter: {:?}",
        t.blocks()
    );
    assert!(
        t.blocks().iter().all(|b| b.text != RETRY_NOTICE),
        "nothing streamed in this chapter, so no notice: {:?}",
        t.blocks()
    );
    // The retry re-streams: its text belongs to chapter 2, never chapter 1.
    apply_event(&mut t, AgentEvent::assistant_text("new answer"), false);
    let new_answer = t.blocks().last().expect("the retry opens its own block");
    assert_eq!(new_answer.kind, BlockKind::Assistant);
    assert_eq!(
        new_answer.chapter,
        2,
        "the retried text must land in the chapter in flight: {:?}",
        t.blocks()
    );
    assert_eq!(new_answer.text, "new answer");
    let old = t
        .blocks()
        .iter()
        .find(|b| b.kind == BlockKind::Assistant && b.chapter == 1)
        .expect("the old answer is still there after the retry");
    assert_eq!(old.text, "old answer");
}

/// The narrow unit behind the audit sequence: a reset with no answer block
/// in the chapter in flight clears nothing anywhere — the transcript is
/// byte-for-byte unchanged, notice included.
#[test]
fn a_reset_with_no_answer_in_this_chapter_clears_nothing() {
    let mut t = Transcript::new();
    t.push(BlockKind::User, "old question");
    apply_event(&mut t, AgentEvent::assistant_text("old answer"), false);
    t.push(BlockKind::User, "new question");
    let before = snapshot(&t);
    apply_event(&mut t, AgentEvent::turn_reset(), false);
    assert_eq!(
        snapshot(&t),
        before,
        "no answer in this chapter → the reset must be a whole-transcript no-op: {:?}",
        t.blocks()
    );
}

/// Positive control (passes before and after the fix): a retry *within* one
/// chapter still discards the partial — the chapter bound must not weaken
/// the discard. Kept as a control, not counted as evidence of the fix.
#[test]
fn a_retry_within_one_chapter_still_discards_the_partial() {
    let mut t = Transcript::new();
    t.push(BlockKind::User, "current question");
    apply_event(&mut t, AgentEvent::assistant_text("partial "), false);
    apply_event(&mut t, AgentEvent::assistant_text("answer"), false);
    apply_event(&mut t, AgentEvent::turn_reset(), false);
    assert!(
        t.blocks().iter().all(|b| !b.text.contains("partial")),
        "the partial in the live chapter is still discarded: {:?}",
        t.blocks()
    );
    let answers = t
        .blocks()
        .iter()
        .filter(|b| b.kind == BlockKind::Assistant)
        .count();
    assert_eq!(
        answers,
        1,
        "one chapter, one answer block: {:?}",
        t.blocks()
    );
    assert!(
        t.blocks().iter().any(|b| b.text == RETRY_NOTICE),
        "the within-chapter retry is still announced: {:?}",
        t.blocks()
    );
    apply_event(&mut t, AgentEvent::assistant_text("the full answer"), false);
    let answer = t
        .blocks()
        .iter()
        .find(|b| b.kind == BlockKind::Assistant)
        .expect("the re-stream resumes the same block");
    assert_eq!(answer.text, "the full answer");
    assert_eq!(
        answer.chapter,
        1,
        "the whole retry stays inside the one chapter: {:?}",
        t.blocks()
    );
}

/// The notice-trailing path (control, passes before and after): a second
/// retry landing after the re-stream began, still inside one chapter, must
/// clear the resumed partial behind the notice and resume into that same
/// block — the behaviour the doc comments on `reset_answer` describe.
#[test]
fn a_second_retry_after_the_restream_began_still_resumes_behind_the_notice() {
    let mut t = Transcript::new();
    t.push(BlockKind::User, "current question");
    apply_event(&mut t, AgentEvent::assistant_text("first try"), false);
    apply_event(&mut t, AgentEvent::turn_reset(), false);
    apply_event(&mut t, AgentEvent::assistant_text("resumed"), false);
    // Second reset while the tail is the notice: the resumed partial sits
    // *behind* it and must still be found and discarded.
    apply_event(&mut t, AgentEvent::turn_reset(), false);
    assert!(
        t.blocks().iter().all(|b| !b.text.contains("resumed")),
        "the resumed partial is discarded by the second retry: {:?}",
        t.blocks()
    );
    let notices = t.blocks().iter().filter(|b| b.text == RETRY_NOTICE).count();
    assert_eq!(
        notices,
        2,
        "each discarded attempt leaves its own notice: {:?}",
        t.blocks()
    );
    // The re-stream resumes the same single assistant block behind the
    // newest notice, all inside chapter 1.
    apply_event(
        &mut t,
        AgentEvent::assistant_text("the final answer"),
        false,
    );
    let answers: Vec<_> = t
        .blocks()
        .iter()
        .filter(|b| b.kind == BlockKind::Assistant)
        .collect();
    assert_eq!(
        answers.len(),
        1,
        "the re-stream never opens a second assistant block: {:?}",
        t.blocks()
    );
    assert_eq!(answers[0].text, "the final answer");
    assert_eq!(answers[0].chapter, 1);
    assert_eq!(
        t.blocks().last().map(|b| b.text.as_str()),
        Some(RETRY_NOTICE),
        "the newest notice trails the resumed answer: {:?}",
        t.blocks()
    );
}
