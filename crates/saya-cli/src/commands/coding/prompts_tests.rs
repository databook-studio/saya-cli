use super::{DIRECT, system, system_with_file_sources};
use crate::agent::file_sources::{FileSources, guidance};
use crate::agent::tools::DatabaseTools;

/// The coding job gets the shared data-file block: the same text the
/// interactive session appends, at the end so the prefix stays cacheable.
#[test]
fn a_checkout_with_data_files_and_duckdb_gets_the_shared_block() {
    let prompt = system_with_file_sources(false, false, FileSources::DuckdbAvailable);
    assert!(
        prompt.starts_with(DIRECT),
        "the block is an append: {prompt}"
    );
    assert!(
        prompt.ends_with(guidance(FileSources::DuckdbAvailable).unwrap()),
        "the block is last: {prompt}"
    );
}

/// A checkout without data files keeps the prompt it always got.
#[test]
fn a_checkout_without_data_files_keeps_the_plain_prompt() {
    assert_eq!(
        system_with_file_sources(false, false, FileSources::NoData),
        system(false, false)
    );
}

/// The missing-CLI variant names the install instead of a command that
/// cannot run.
#[test]
fn the_missing_cli_variant_reaches_the_coding_prompt() {
    let prompt = system_with_file_sources(false, false, FileSources::DuckdbMissing);
    assert!(prompt.contains("not on PATH"), "{prompt}");
    assert!(!prompt.contains("duckdb -readonly"), "{prompt}");
}

/// Plan mode is opt-in, and the prompt proves it: a job that never stated
/// `--plan` gets the direct prompt byte for byte, with no plan sentence
/// anywhere in it.
#[test]
fn a_job_without_plan_mode_gets_the_direct_prompt_unchanged() {
    assert_eq!(
        system(false, false),
        DIRECT,
        "a job without plan mode gets the prompt it always did"
    );
    for sentence in ["plan-first", "`submit_plan`", "the write surface is closed"] {
        assert!(
            !system(false, false).contains(sentence),
            "no plan posture in a job that never stated `--plan`: {sentence}"
        );
    }
}

/// A plan-first job is told the posture it is under, what ends it, and that
/// nothing has changed yet — and it is told all of it in addition to the direct
/// prompt, not instead of it.
#[test]
fn a_plan_first_job_is_told_the_posture_and_what_ends_it() {
    let prompt = system(true, false);
    for sentence in [
        "This job is plan-first: the write surface is closed, so nothing can be written, edited or \
         deleted and no command runs until the operator approves a plan.",
        "Read what you need, then propose the plan you would carry out and end that turn with \
         `submit_plan` and nothing after it.",
        "`submit_plan` is the whole of the question: the operator answers it, and their approval \
         opens the write surface for the rest of this job while a refusal leaves it closed and you \
         revise the plan and submit it again.",
        "Do not describe work as done before the surface opens; nothing has been changed yet.",
    ] {
        assert!(
            prompt.contains(sentence),
            "the plan prompt must state this: {sentence} in {prompt}"
        );
    }
    assert!(
        prompt.contains("The user's request is the only authority for requirements."),
        "the plan prompt is the direct prompt plus the posture: {prompt}"
    );
    assert_ne!(prompt, DIRECT, "the two postures are different prompts");
}

/// Batching is the default: the prompt asks for independent calls in one
/// turn, and the `edit` description does not send whole-file work to `write`
/// (the sentence that produced one-file-per-turn re-emissions). Both
/// halves are model-facing text for the same behaviour, so they are pinned
/// together.
#[test]
fn direct_prompt_batches_independent_calls_and_edit_does_not_steer_to_write() {
    let prompt = system(false, false);
    assert!(
        prompt.contains(
            "When calls do not depend on each other — several reads, several new files, a read \
             plus a search — emit them in the same turn."
        ),
        "the direct prompt must ask for independent calls in one turn: {prompt}"
    );
    let tools = DatabaseTools::definitions(false, false, false, true);
    let edit = tools
        .iter()
        .find(|tool| tool.name == "edit")
        .expect("edit is advertised when workspace writes are permitted");
    assert!(
        !edit.description.contains("use `write` instead"),
        "edit must not steer whole-file writes to `write`: {}",
        edit.description
    );
    assert!(
        edit.description
            .contains("Replace `old_text` with `new_text` in a file."),
        "edit must state the change plainly: {}",
        edit.description
    );
    let write = tools
        .iter()
        .find(|tool| tool.name == "write")
        .expect("write is advertised when workspace writes are permitted");
    assert!(
        write.description.contains("Write a whole file."),
        "write must state the whole-file split plainly: {}",
        write.description
    );
}

/// The loop-breaking sentences: the prompt names the three observed loops —
/// re-`write` of a path this run wrote, `read`-back of a file this run wrote,
/// and one test file per turn — concretely and imperatively. The read-back
/// sentence leans on the `write` result carrying the digest an `edit` takes,
/// so that claim is pinned
/// against the tool text it describes.
#[test]
fn direct_prompt_names_the_rewrite_read_back_and_retest_loops() {
    let prompt = system(false, false);
    for sentence in [
        "After you create a file, change it with `edit`; do not `write` that path again unless \
         you are replacing most of it.",
        "Do not `read` a file you just wrote: it holds what you sent, and the `write` result \
         gives the digest `edit` needs.",
        "When the task changes code, fix every failure you can see before re-running tests, and \
         verify with the project's own build and tests; do not write extra probe, fuzz or \
         measurement scripts unless the request asks for them.",
    ] {
        assert!(
            prompt.contains(sentence),
            "the direct prompt must name this loop: {sentence} in {prompt}"
        );
    }
    let tools = DatabaseTools::definitions(false, false, false, true);
    let write = tools
        .iter()
        .find(|tool| tool.name == "write")
        .expect("write is advertised when workspace writes are permitted");
    assert!(
        write.description.contains("Returns `digest`"),
        "the read-back sentence is only true while `write` returns the digest: {}",
        write.description
    );
    let edit = tools
        .iter()
        .find(|tool| tool.name == "edit")
        .expect("edit is advertised when workspace writes are permitted");
    assert!(
        edit.description.contains("`expected_digest`"),
        "the read-back sentence is only true while `edit` takes that digest: {}",
        edit.description
    );
}

/// Verification is the project's own build and tests, in both postures: the
/// root must not spend turns writing probe, fuzz or measurement scripts the
/// request never asked for. The plan posture is [`DIRECT`] plus the posture
/// sentences, so the sentence has to survive the splice in both.
#[test]
fn direct_prompt_verifies_with_the_projects_own_tests() {
    for (posture, prompt) in [
        ("direct", system(false, false)),
        ("plan-first", system(true, false)),
    ] {
        assert!(
            prompt.contains(
                "do not write extra probe, fuzz or measurement scripts unless the request asks \
                 for them"
            ),
            "the {posture} prompt must send verification to the project's own tests: {prompt}"
        );
    }
}

/// Rule (a) 1: the operator's request is the only authority for requirements.
#[test]
fn direct_prompt_makes_the_request_the_only_authority() {
    let prompt = system(false, false);
    assert!(
        prompt.contains("The user's request is the only authority for requirements."),
        "the direct prompt must make the request authoritative: {prompt}"
    );
}

/// Rule (a) 2: no invented requirements, formats or conventions where the
/// request leaves room; prefer the most conventional reading.
#[test]
fn direct_prompt_forbids_inventing_requirements_where_the_request_leaves_room() {
    let prompt = system(false, false);
    assert!(
        prompt.contains(
            "Do not invent requirements, formats or conventions where the request leaves room; \
             prefer the most conventional reading."
        ),
        "the direct prompt must forbid invented requirements: {prompt}"
    );
}

/// Rule (a) 3: an interpretation the assignment adds is marked as the root's
/// own reading and quotes the request sentence it interprets.
#[test]
fn direct_prompt_requires_marking_an_added_interpretation_as_my_reading() {
    let prompt = system(false, false);
    assert!(
        prompt.contains(
            "When an assignment adds an interpretation of the request, mark it explicitly as \
             'my reading' and quote the request sentence it interprets."
        ),
        "the direct prompt must require the 'my reading' marker: {prompt}"
    );
}

/// The working list is offered, not mandated: a job that never needs one must
/// not be told it has to keep one.
#[test]
fn direct_prompt_leaves_the_working_list_optional() {
    let prompt = system(false, false);
    assert!(
        prompt.contains("Use `checklist` only when tracking several steps helps."),
        "the direct prompt must offer the working list: {prompt}"
    );
    assert!(
        !prompt.contains("You must use `checklist`"),
        "the working list is not a mandate: {prompt}"
    );
}

/// The prompt must not describe what the owner does with checks — no checks
/// run — and must not inventory the tools: the tool definitions are the only
/// statement of what this run was given, so a list here drifts false whenever
/// shell or writes are not offered.
#[test]
fn direct_prompt_states_no_checks_and_lists_no_tools() {
    let prompt = system(false, false);
    assert!(
        prompt.contains(
            "The owner prepares the candidate and its readiness from your checkout after the \
             job ends."
        ),
        "the direct prompt must state what the owner still does: {prompt}"
    );
    for mention in ["checks", "check the candidate", "check profile"] {
        assert!(
            !prompt.contains(mention),
            "checks are gone, so no statement may mention them: {mention} in {prompt}"
        );
    }
    for listed in [
        "read, list, glob, grep, edit, write, delete, and shell",
        "`shell`",
        "`delete`",
    ] {
        assert!(
            !prompt.contains(listed),
            "the tool inventory drifts false when a tool is not offered: {listed} in {prompt}"
        );
    }
}

/// The job is not necessarily code: the prompt names the agent generally and
/// makes every build-and-test sentence conditional on the task changing code.
/// A documentation or analysis job is told to check its output against the
/// request instead.
#[test]
fn direct_prompt_addresses_a_generic_job_not_only_a_coding_one() {
    let prompt = system(false, false);
    assert!(
        prompt.contains("You are the agent for this job, which may change code, write documents or analyse files."),
        "the prompt must address the job generally: {prompt}"
    );
    assert!(
        !prompt.contains("coding agent"),
        "the prompt must not call itself a coding agent: {prompt}"
    );
    for sentence in [
        "When the task changes code, fix every failure you can see before re-running tests",
        "When the task does not change code, check your output against the request.",
        "When the task changes code, run the narrowest test target while iterating; run the full suite once before finishing.",
    ] {
        assert!(
            prompt.contains(sentence),
            "the prompt must state this condition: {sentence} in {prompt}"
        );
    }
}

/// One test rule, not two: the F1 rule is the only statement about how much
/// to run while iterating, so the older whole-suite-per-call sentence is gone.
#[test]
fn direct_prompt_states_one_test_rule() {
    let prompt = system(false, false);
    assert!(
        !prompt.contains("re-run the whole suite in one call"),
        "the contradictory whole-suite rule is gone: {prompt}"
    );
    assert!(
        !prompt.contains("not one file per turn"),
        "the contradictory per-file rule is gone: {prompt}"
    );
    assert_eq!(
        prompt
            .matches("run the full suite once before finishing")
            .count(),
        1,
        "the surviving rule is stated once: {prompt}"
    );
}

/// The batching and bounded-verification rules reach the direct prompt only,
/// and each is stated for code tasks — the read-only prompt changes no code,
/// so it carries neither.
#[test]
fn direct_prompt_states_the_batching_and_bounded_verification_rules() {
    const BATCH: &str = "When code changes, read every file and call site the change touches \
         before the first edit, then put all of a file's changes in one `edit` call (`edits` for \
         several hunks). ";
    const VERIFY: &str =
        "Once code tests pass on a small diff, do not re-run the suite or add extra tests. ";
    for sentence in [BATCH, VERIFY] {
        assert!(
            DIRECT.contains(sentence),
            "the direct prompt states: {sentence}"
        );
        assert!(system(false, false).contains(sentence));
        assert!(
            !system(false, true).contains(sentence),
            "the read-only prompt carries no code-task rule: {sentence}"
        );
    }
}

#[test]
fn direct_prompt_makes_delegation_optional() {
    let prompt = system(false, false);
    for hidden in ["`helpers`", "`coding_check`", "`coding_skills`"] {
        assert!(
            !prompt.contains(hidden),
            "the direct prompt must not name the hidden first-ship tool {hidden}: {prompt}"
        );
    }
    for mandate in [
        "delegate one implementing child once",
        "Follow the next_action",
        "register the complete intended child assignment set",
    ] {
        assert!(
            !prompt.contains(mandate),
            "the direct prompt must not carry the delegate-only mandate: {mandate}"
        );
    }
}
