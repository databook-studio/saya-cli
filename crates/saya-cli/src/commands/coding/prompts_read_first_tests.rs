use super::{DIRECT, READ_ONLY, system};

/// Both postures ask for a short final answer: what changed and how it was
/// verified, never a restatement of the request or a file-by-file narration.
const SHORT_FINAL: &str = "When you are done, end your turn with a short final answer: what \
     changed and how you verified it; do not restate the request or narrate file by file. ";

/// The code-change rule: independent edits in the same turn, verified with
/// tests rather than a re-read.
const BATCH_EDITS: &str = "When code changes, after reading make independent edits in the same \
     turn, then verify with the tests rather than re-reading. ";

#[test]
fn both_prompts_ask_for_a_short_final_answer() {
    assert!(
        DIRECT.contains(SHORT_FINAL),
        "the direct prompt states: {SHORT_FINAL}"
    );
    assert!(
        READ_ONLY.contains(SHORT_FINAL),
        "the read-only prompt states: {SHORT_FINAL}"
    );
}

#[test]
fn the_direct_prompt_batches_edits_and_verifies_with_tests() {
    assert!(
        DIRECT.contains(BATCH_EDITS),
        "the direct prompt states: {BATCH_EDITS}"
    );
    assert!(
        !READ_ONLY.contains(BATCH_EDITS),
        "the read-only prompt changes no code, so it carries no edit rule"
    );
}

/// The new lines are a small addition: each prompt stays within 250 bytes of
/// its pre-change length (DIRECT 2371, READ_ONLY 933).
#[test]
fn the_prompt_additions_stay_within_their_byte_budget() {
    assert!(
        DIRECT.len() <= 2371 + 250,
        "the direct prompt grew to {} bytes",
        DIRECT.len()
    );
    assert!(
        READ_ONLY.len() <= 933 + 250,
        "the read-only prompt grew to {} bytes",
        READ_ONLY.len()
    );
}

/// The no-read-back rule is stated for code tasks only: after an `edit` the
/// result already carries the file's digest, so reading the file back spends
/// a turn. The read-only prompt cannot edit, so it carries no such rule.
#[test]
fn direct_prompt_does_not_read_back_after_an_edit() {
    const NO_READ_BACK: &str =
        "Do not `read` a file you just edited: the edit result already has its digest. ";
    assert!(
        DIRECT.contains(NO_READ_BACK),
        "the direct prompt states: {NO_READ_BACK}"
    );
    assert!(system(false, false).contains(NO_READ_BACK));
    assert!(
        !system(false, true).contains(NO_READ_BACK),
        "the read-only prompt carries no code-task rule: {NO_READ_BACK}"
    );
}
