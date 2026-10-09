use super::*;

/// A no-digest publish to an absent path creates the file — the create form.
#[test]
fn publish_creates_a_new_file_without_a_digest() {
    let sandbox = SaveWorkspace::new("create");
    publish_chart_html(&sandbox.workspace, "chart.html", b"<html></html>", None)
        .expect("a no-digest publish to an absent path must create the file");
    assert_eq!(
        fs::read(sandbox.root.join("chart.html")).expect("file must exist"),
        b"<html></html>"
    );
}

/// The defining refusal: publishing over an existing file without a digest
/// changes nothing and surfaces the same text `workspace_write` surfaces.
#[test]
fn publish_refuses_over_an_existing_file_without_a_digest() {
    let sandbox = SaveWorkspace::new("no-digest");
    sandbox.seed("chart.html", b"original bytes");
    let error = publish_chart_html(&sandbox.workspace, "chart.html", b"<html></html>", None)
        .expect_err("replacing an existing file without a digest must refuse");
    match error {
        ToolError::WorkspaceWrite(message) => {
            assert!(
                message.contains("expected_digest") && message.contains("workspace_read"),
                "the refusal must tell the model how to replace: {message}"
            );
        }
        other => panic!("expected a typed workspace-write error, got: {other}"),
    }
    assert_eq!(
        fs::read(sandbox.root.join("chart.html")).expect("file must survive"),
        b"original bytes",
        "the file must be byte-identical after the refused publish"
    );
}

/// A stale digest refuses: the file moved since it was observed, and the
/// other writer's bytes survive — never last-writer-wins.
#[test]
fn publish_refuses_a_stale_digest_and_preserves_the_bytes() {
    let sandbox = SaveWorkspace::new("stale");
    sandbox.seed("chart.html", b"original bytes");
    let observed = sandbox.observed_digest("chart.html");
    sandbox
        .workspace
        .write("chart.html", b"edited bytes")
        .expect("the external overwrite must land");
    let error = publish_chart_html(
        &sandbox.workspace,
        "chart.html",
        b"<html></html>",
        Some(&observed),
    )
    .expect_err("a stale digest must refuse the replace");
    match error {
        ToolError::WorkspaceWrite(message) => {
            assert!(
                message.contains("changed") && message.contains("workspace_read"),
                "the refusal must send the model back to a fresh read: {message}"
            );
        }
        other => panic!("expected a typed workspace-write error, got: {other}"),
    }
    assert_eq!(
        fs::read(sandbox.root.join("chart.html")).expect("file must survive"),
        b"edited bytes",
        "the other writer's bytes must survive a refused stale publish"
    );
}

/// With the digest the model observed, an unchanged file is replaced whole.
#[test]
fn publish_replaces_an_existing_file_with_the_observed_digest() {
    let sandbox = SaveWorkspace::new("replace");
    sandbox.seed("chart.html", b"original bytes");
    let observed = sandbox.observed_digest("chart.html");
    publish_chart_html(
        &sandbox.workspace,
        "chart.html",
        b"<html></html>",
        Some(&observed),
    )
    .expect("a digest-guarded replace of an unchanged file must succeed");
    assert_eq!(
        fs::read(sandbox.root.join("chart.html")).expect("file must exist"),
        b"<html></html>"
    );
}

/// A digest-carrying publish to an absent path is refused, never a create,
/// and creates nothing — not even the parent directory.
#[test]
fn publish_refuses_a_digest_for_a_missing_file() {
    let sandbox = SaveWorkspace::new("digest-missing");
    let error = publish_chart_html(
        &sandbox.workspace,
        "notes/chart.html",
        b"<html></html>",
        Some(&"0".repeat(64)),
    )
    .expect_err("a digest-carrying publish to a missing file must refuse");
    match error {
        ToolError::WorkspaceWrite(message) => {
            assert!(
                message.contains("does not exist") && message.contains("expected_digest"),
                "the refusal must point at the digest-vs-create confusion: {message}"
            );
        }
        other => panic!("expected a typed workspace-write error, got: {other}"),
    }
    assert!(
        !sandbox.root.join("notes").exists(),
        "a refused digest publish must create nothing, not even the parent"
    );
}

/// Chart HTML over the whole-file I/O bound refuses before any filesystem
/// contact — the chart's own bound, not the 64 KiB model-write bound.
#[test]
fn publish_refuses_over_bound_html() {
    let sandbox = SaveWorkspace::new("bound");
    let oversized = vec![b'x'; MAX_IO_BYTES + 1];
    let error = publish_chart_html(&sandbox.workspace, "chart.html", &oversized, None)
        .expect_err("over the bound must be refused whole");
    match error {
        ToolError::WorkspaceWrite(message) => {
            assert!(
                message.contains("over the") && message.contains(&MAX_IO_BYTES.to_string()),
                "the refusal must carry the chart's own bound: {message}"
            );
        }
        other => panic!("expected a typed workspace-write error, got: {other}"),
    }
    assert!(
        !sandbox.root.join("chart.html").exists(),
        "a refused publish must create nothing"
    );
}
