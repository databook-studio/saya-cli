use super::*;

/// Property 6: failure detection agrees with `tool_metadata.status` — both
/// key on the summary containing "failed".
#[test]
fn failure_detection_agrees_with_tool_metadata_status() {
    for summary in ["failed pytest", "failed to complete: host command ran"] {
        assert!(
            is_failure_summary(summary),
            "the grouper must classify {summary:?} as failed"
        );
        let status = if summary.contains("failed") {
            "failed"
        } else {
            "completed"
        };
        assert_eq!(
            status, "failed",
            "tool_metadata.status derives from the same predicate"
        );
    }
    for summary in ["pytest exited 1", "notes.md written"] {
        assert!(
            !is_failure_summary(summary),
            "the grouper must not classify {summary:?} as failed"
        );
    }
}

/// H2 reproduction (must fail before the fix): a multi-call group whose
/// `run_command` member completed with a nonzero exit (`pytest exited 1`,
/// an `Ok` outcome, no "failed" substring) must not shape into the all-ok
/// header — the failure must be visible on the piped surface, with the
/// evidence (the exit code) carried in the default surface.
#[test]
fn a_nonzero_exit_does_not_read_as_ok_on_the_piped_surface() {
    let events = vec![
        requested(
            "workspace_write",
            serde_json::json!({"path": "notes.md"}),
            write_effect(),
        ),
        completed("workspace_write", "notes.md written"),
        requested(
            "run_command",
            serde_json::json!({"program": "pytest", "args": ["-q"]}),
            run_effect(),
        ),
        completed("run_command", "pytest exited 1"),
    ];
    let groups = group_tool_events(&events);
    assert_eq!(groups.len(), 1);
    let shaped = shape_group(&groups[0]);
    let rendered = shaped.join("\n");
    assert!(
        !rendered.contains("· ok"),
        "a nonzero exit must not shape into the all-ok header: {rendered:?}"
    );
    assert!(
        rendered.contains("exited 1"),
        "the exit code must survive on the default surface: {rendered:?}"
    );
}
