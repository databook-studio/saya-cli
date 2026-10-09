/// The README's `/context` sample is a real capture: the README's command
/// line runs verbatim, and every quoted sentence appears in the actual
/// output — and in the README itself, so a fake quote cannot pass.
#[test]
fn readme_context_sample_matches_the_real_binary() {
    let env = env("context-sample");
    std::fs::write(
        env.project.join("AGENTS.md"),
        "Run cargo fmt before committing.\n",
    )
    .unwrap();
    std::fs::write(
        env.root.join("saya/instructions.md"),
        "Personal: keep answers short.\n",
    )
    .unwrap();
    let output = context_turn(&env);
    exit_zero(&output, "the /context turn");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    for sentence in README_CONTEXT_SENTENCES {
        assert!(
            stdout.contains(sentence),
            "the README quotes {sentence:?}; the real output must carry it: {stdout}"
        );
        assert!(
            README.contains(sentence),
            "the sentence {sentence:?} must stay quoted in the README sample"
        );
    }
    let _ = std::fs::remove_dir_all(&env.root);
}

/// The caps the README states: a project AGENTS.md over 32 KiB is included
/// only up to the cap, marked truncated, with the whole-file size and digest
/// still reported.
#[test]
fn an_oversize_agent_md_is_reported_truncated() {
    let env = env("oversize");
    std::fs::write(env.project.join("AGENTS.md"), "x".repeat(40_000)).unwrap();
    let output = context_turn(&env);
    exit_zero(&output, "the /context turn");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let receipt = "project AGENTS.md — 40000 B file, 32768 B included, sha256 ";
    assert!(
        stdout.contains(receipt),
        "the whole-file size and the capped inclusion are both shown: {stdout}"
    );
    assert!(
        stdout.contains("truncated: yes"),
        "truncation is marked: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}
