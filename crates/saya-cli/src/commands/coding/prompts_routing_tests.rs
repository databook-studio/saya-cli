//! F1: the root prompt's tool routing. The lines are model-facing text for
//! behaviour measured in `ref/REPORT-S3S4.md` (RC1, RC3, RC4): dedicated
//! file tools instead of shell `cat`/`grep`/`sed`, fresh digests, no `cd`
//! prefix, and the narrowest test target while iterating. Each line is pinned
//! verbatim, and the block they form is bounded.

use super::{system, system_with_file_sources};
use crate::agent::file_sources::{FileSources, guidance};

/// The routing lines, exactly as the prompt states them.
const ROUTING: [&str; 5] = [
    "Use `read`, `grep` and `glob` for files and `edit` to change them; use the shell for builds, \
     tests and git.",
    "Batch independent reads and searches in one turn.",
    "Take `expected_digest` from the most recent read, write or edit of that exact path.",
    "The shell already starts in the working checkout root; do not prefix commands with `cd`.",
    "When the task changes code, run the narrowest test target while iterating; run the full suite \
     once before finishing.",
];

/// The background line, exactly as the prompt states it (F2): a long build or
/// test is started detached so the loop keeps working, and read back by id.
const BACKGROUND: &str = "When the task changes code, start a long build or test with \
     `run_in_background`, keep working, then read it with `shell_output`.";

/// The routing lines reach both postures: the plan posture is [`super::DIRECT`]
/// plus the posture sentences, so the block must survive the splice.
#[test]
fn both_postures_state_the_tool_routing_lines() {
    for (posture, prompt) in [
        ("direct", system(false, false)),
        ("plan-first", system(true, false)),
    ] {
        for line in ROUTING {
            assert!(
                prompt.contains(line),
                "the {posture} prompt must state this routing line: {line} in {prompt}"
            );
        }
    }
}

/// F2: both postures tell the model to start a long build or test detached,
/// keep working, and read the result by id.
#[test]
fn both_postures_route_long_builds_to_the_background() {
    for (posture, prompt) in [
        ("direct", system(false, false)),
        ("plan-first", system(true, false)),
    ] {
        assert!(
            prompt.contains(BACKGROUND),
            "the {posture} prompt must state the background line: {BACKGROUND} in {prompt}"
        );
    }
}

/// The data-file block is the append that must stay last: no routing line may
/// trail it, or the cacheable prefix changes with the checkout's data files.
#[test]
fn the_routing_lines_come_before_the_file_source_block() {
    let prompt = system_with_file_sources(false, false, FileSources::DuckdbAvailable);
    let block = guidance(FileSources::DuckdbAvailable).expect("duckdb is available");
    assert!(prompt.ends_with(block), "the block is last: {prompt}");
    let before = &prompt[..prompt.len() - block.len()];
    for line in ROUTING {
        assert!(
            before.contains(line),
            "routing lines precede the block: {line} in {prompt}"
        );
    }
}

/// The block is short by contract: the routing lines themselves add at most
/// 700 bytes to the prompt. Each line is stated followed by one space, so
/// the block's size is the sum of the lines plus their separators; bounding
/// the block itself keeps the budget meaningful as later rules join the
/// prompt.
#[test]
fn the_routing_lines_grow_the_prompt_by_at_most_700_bytes() {
    let block: usize = ROUTING.iter().map(|line| line.len() + 1).sum();
    assert!(
        block <= 700,
        "the routing block is {block} bytes, over the 700-byte budget"
    );
}
