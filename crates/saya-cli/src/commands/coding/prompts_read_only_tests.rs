use super::{DIRECT, system};

/// A read-only job is told the truth: it reads and searches its own folder,
/// and the write, edit, shell and owner-candidate sentences are absent.
/// A read-only job may be code, documentation or a question, so it is
/// addressed as a job and told to check its answer against the request.
#[test]
fn a_read_only_job_is_addressed_generically() {
    let prompt = system(false, true);
    assert!(
        prompt.contains(
            "You are the agent for this job, which may read code, documents or data files."
        ),
        "the read-only prompt must address the job generally: {prompt}"
    );
    assert!(
        prompt.contains("Check your answer against the request."),
        "a read-only job verifies its output against the request: {prompt}"
    );
    assert!(
        !prompt.contains("coding agent"),
        "the read-only prompt must not call itself a coding agent: {prompt}"
    );
}

#[test]
fn a_read_only_job_gets_the_read_only_prompt() {
    let prompt = system(false, true);
    for sentence in [
        "This job is read-only: you can read and search the workspace, but you cannot write, edit \
         or delete files and you have no shell.",
        "Use `read`, `grep` and `glob` for files.",
    ] {
        assert!(
            prompt.contains(sentence),
            "the read-only prompt must state this: {sentence} in {prompt}"
        );
    }
    for absent in [
        "You have workspace tools over your own working checkout.",
        "`edit` to change them",
        "The shell already starts in the working checkout root",
        "The owner prepares the candidate and its readiness from your checkout after the job ends.",
        "`submit_plan`",
    ] {
        assert!(
            !prompt.contains(absent),
            "a read-only job is not told this: {absent} in {prompt}"
        );
    }
    assert_ne!(prompt, DIRECT, "the read-only prompt is its own prompt");
}
