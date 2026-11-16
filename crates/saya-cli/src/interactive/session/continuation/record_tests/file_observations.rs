/// A forged assistant summary mints nothing: the collector reads observed
/// tool activity, never the model's prose — even prose that mimics a
/// completion summary's exact shapes.
#[test]
fn a_forged_summary_produces_no_facts() {
    let mut observations = SessionObservations::default();
    observations.absorb(
        &output(
            "I edited src/x.rs and all tests passed (cargo exited 0)",
            Vec::new(),
            Vec::new(),
        ),
        NOW,
    );
    assert!(
        observations.files.is_empty(),
        "prose minted a file fact: {:?}",
        observations.files
    );
    assert!(
        observations.checks.is_empty(),
        "prose minted a check fact: {:?}",
        observations.checks
    );
}

/// Completed write-shaped tool calls become file observations with tool
/// provenance; failed, denied, and read-shaped tools record nothing.
#[test]
fn workspace_tools_that_completed_produce_file_observations() {
    let mut observations = SessionObservations::default();
    observations.absorb(
        &output(
            "done",
            vec![
                tool("edit", "completed", r#"{"path":"src/a.rs"}"#),
                tool("write", "completed", r#"{"path":"docs/b.md"}"#),
                tool("edit", "failed", r#"{"path":"src/c.rs"}"#),
                tool("write", "denied", r#"{"path":"src/d.rs"}"#),
                tool("read", "completed", r#"{"path":"src/e.rs"}"#),
            ],
            Vec::new(),
        ),
        NOW,
    );
    assert_eq!(
        observations.files,
        vec![
            super::FileObservation {
                path: "src/a.rs".into(),
                tool: "edit".into(),
                observed_unix_ms: NOW,
            },
            super::FileObservation {
                path: "docs/b.md".into(),
                tool: "write".into(),
                observed_unix_ms: NOW,
            },
        ],
        "only completed write-shaped calls name a file"
    );
    assert!(observations.checks.is_empty());
}

/// Absolute, escaping, non-canonical, and control-character paths are
/// dropped: the collector enforces the same relative-path rule the contract
/// validates, so an invalid observation can never reach a fact.
#[test]
fn an_absolute_or_escaping_path_is_dropped() {
    let mut observations = SessionObservations::default();
    observations.absorb(
        &output(
            "done",
            vec![
                tool("write", "completed", r#"{"path":"/etc/passwd"}"#),
                tool("write", "completed", r#"{"path":"../x"}"#),
                tool("write", "completed", r#"{"path":"a//b"}"#),
                tool("write", "completed", r#"{"path":"nul\u0000x"}"#),
                tool("write", "completed", r#"{"path":"C:\x"}"#),
                tool("write", "completed", r#"{"path":""}"#),
            ],
            Vec::new(),
        ),
        NOW,
    );
    assert!(
        observations.files.is_empty(),
        "only relative canonical paths are observed: {:?}",
        observations.files
    );
}
