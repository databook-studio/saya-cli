/// Drift guard: every `saya …` line in a README ```bash block must be in one
/// of the registers above (run / derived with its reason / skipped with its
/// reason), and every register entry must still appear in the README — so a
/// changed command fails here until the register is updated too.
#[test]
fn readme_bash_lines_are_run_derived_or_explicitly_skipped() {
    let mut in_bash = false;
    let mut seen: Vec<&str> = Vec::new();
    for line in README.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("```") {
            in_bash = !in_bash && rest == "bash";
            continue;
        }
        if in_bash && trimmed.starts_with("saya ") {
            seen.push(trimmed);
        }
    }
    assert!(!seen.is_empty(), "the README must show commands");
    for line in &seen {
        let accounted = README_RUNNABLE.contains(line)
            || README_DERIVED
                .iter()
                .any(|(registered, _)| registered == line)
            || README_SKIPPED
                .iter()
                .any(|(registered, _)| registered == line);
        assert!(
            accounted,
            "README bash line {line:?} is neither run by these tests nor \
             listed with a reason in the run/derived/skipped registers"
        );
    }
    for line in README_RUNNABLE {
        assert!(
            seen.contains(line),
            "registered runnable line {line:?} is no longer in the README"
        );
    }
    for (registered, _) in README_DERIVED.iter().chain(README_SKIPPED.iter()) {
        assert!(
            seen.contains(registered),
            "registered line {registered:?} is no longer in the README"
        );
    }
}
