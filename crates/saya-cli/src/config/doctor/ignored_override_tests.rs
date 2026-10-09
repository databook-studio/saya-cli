use super::ignored_override_lines;

/// The one-shot warning is deliberately terse, so doctor is the only place
/// a user can learn *which* settings were ignored. If this stops reporting
/// them, the terse warning becomes a dead end.
#[test]
fn doctor_names_every_ignored_setting_and_the_way_to_apply_it() {
    let report = ignored_override_lines(&["ai.base_url".to_string(), "run.read_only".to_string()])
        .join("\n");

    assert!(
        report.contains("ai.base_url") && report.contains("run.read_only"),
        "doctor must name each ignored setting: {report}"
    );
    // Both routes: the trusted one (move them) and the override.
    assert!(
        report.contains("user config") && report.contains("--trust-project-config"),
        "doctor must say how to apply them: {report}"
    );
}

/// A project layer that changed nothing adds no noise to the report.
#[test]
fn doctor_is_silent_when_nothing_was_ignored() {
    assert!(
        ignored_override_lines(&[]).is_empty(),
        "no ignored settings means no section"
    );
}
