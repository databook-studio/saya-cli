//! Tests for the `/context` renderer: the one shared view the headless loop
//! and the TUI dispatch both print. The contract under test: per admitted
//! source — origin, file name, whole-file size, bytes included, short
//! digest, truncation and redaction flags; per skipped source — origin and
//! reason; per omitted source — origin and the budget reason; plus the
//! effective settings. The source bodies themselves must never appear.

use super::render;
use crate::agent::context::sources::project_instructions;
use saya_config::ResolvedInstructions;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const SENTINEL: &str = "SENTINEL-INSTRUCTION-BODY";

fn stamp(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "saya-context-view-{tag}-{}-{nanos}",
        std::process::id()
    ))
}

fn write_file(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn settings() -> ResolvedInstructions {
    ResolvedInstructions {
        project_enabled: true,
        personal_enabled: true,
        project_max_bytes: 32 * 1024,
        personal_max_bytes: 16 * 1024,
        total_max_bytes: 48 * 1024,
    }
}

/// First 12 lowercase hex characters of the file's SHA-256 — exactly what
/// the view is allowed to show of a source's identity.
fn sha12(contents: &str) -> String {
    let digest = Sha256::digest(contents.as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    hex[..12].to_string()
}

fn project_body() -> String {
    format!("project guidance rules with {SENTINEL}\n")
}

fn personal_body() -> String {
    format!("personal guidance rules with {SENTINEL}\n")
}

#[test]
fn both_admitted_sources_render_names_sizes_digest_and_flags() {
    let root = stamp("both");
    let personal = root.join("personal");
    let project = root.join("project");
    let project_body = project_body();
    let personal_body = personal_body();
    write_file(&personal.join("instructions.md"), &personal_body);
    write_file(&project.join("AGENTS.md"), &project_body);
    let projection = project_instructions(&settings(), Some(&project), &personal, 48 * 1024);
    let view = render(&settings(), &projection);
    assert!(
        view.contains("Loaded:"),
        "admitted sources are listed: {view}"
    );
    assert!(view.contains("project AGENTS.md"), "{view}");
    assert!(view.contains("personal instructions.md"), "{view}");
    assert!(
        view.contains(&format!("{} B file", project_body.len())),
        "the whole-file size is shown: {view}"
    );
    assert!(
        view.contains(&format!("{} B included", project_body.len())),
        "the included byte count is shown: {view}"
    );
    assert!(
        view.contains(&sha12(&project_body)),
        "the short digest is shown: {view}"
    );
    assert!(view.contains("truncated: no"), "{view}");
    assert!(view.contains("redactions: 0"), "{view}");
    assert!(
        view.contains("never permissions") && view.contains("never shown here"),
        "the guidance-never-permissions note is stated: {view}"
    );
    assert!(
        !view.contains(SENTINEL),
        "the source body must never appear anywhere in the view: {view}"
    );
    assert!(
        !view.contains("SENTIN"),
        "no fragment of the body leaks: {view}"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_oversize_source_is_marked_truncated_with_its_full_size() {
    let root = stamp("truncated");
    let personal = root.join("personal");
    let project = root.join("project");
    let body = "héllo — ünïcode — ".repeat(20);
    write_file(&project.join("AGENTS.md"), &body);
    let mut limited = settings();
    limited.personal_enabled = false;
    limited.project_max_bytes = 64;
    let projection = project_instructions(&limited, Some(&project), &personal, 48 * 1024);
    let view = render(&limited, &projection);
    assert!(view.contains("truncated: yes"), "{view}");
    assert!(
        view.contains(&format!("{} B file", body.len())),
        "the whole-file size is shown even when truncated: {view}"
    );
    assert!(view.contains(&sha12(&body)), "{view}");
    let included = projection.sources[0].included_bytes;
    assert!(included < body.len(), "the cap actually trimmed the body");
    assert!(
        view.contains(&format!("{included} B included")),
        "the included byte count matches what the request would carry: {view}"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn a_skipped_symlink_is_named_with_its_reason() {
    use std::os::unix::fs::symlink;
    let root = stamp("skipped");
    let personal = root.join("personal");
    let project = root.join("project");
    let outside = root.join("outside.md");
    write_file(&outside, "secret rules outside the workspace\n");
    write_file(&personal.join("instructions.md"), "personal body\n");
    std::fs::create_dir_all(&project).unwrap();
    symlink(&outside, project.join("AGENTS.md")).unwrap();
    let projection = project_instructions(&settings(), Some(&project), &personal, 48 * 1024);
    assert_eq!(
        projection.skipped.len(),
        1,
        "the symlinked source is skipped"
    );
    let view = render(&settings(), &projection);
    assert!(view.contains("Skipped:"), "{view}");
    assert!(view.contains("project AGENTS.md"), "{view}");
    assert!(view.contains("symlink refused"), "{view}");
    assert!(
        !view.contains("secret rules"),
        "no text from the refused target leaks: {view}"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_omitted_source_is_named_with_the_budget_reason() {
    let root = stamp("omitted");
    let personal = root.join("personal");
    let project = root.join("project");
    // Large enough that no trimmed prefix fits the tiny budget: the source
    // loads but the request has room for none of it.
    write_file(&project.join("AGENTS.md"), &"guidance ".repeat(2 * 1024));
    let mut off = settings();
    off.personal_enabled = false;
    let projection = project_instructions(&off, Some(&project), &personal, 600);
    assert_eq!(
        projection.omitted.len(),
        1,
        "a 600-byte budget omits the source"
    );
    let view = render(&off, &projection);
    assert!(view.contains("Omitted:"), "{view}");
    assert!(view.contains("project AGENTS.md"), "{view}");
    assert!(view.contains("no byte budget"), "{view}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn nothing_found_is_said_plainly_and_the_settings_are_shown() {
    let root = stamp("nothing");
    let personal = root.join("personal");
    let project = root.join("project");
    std::fs::create_dir_all(&personal).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    let projection = project_instructions(&settings(), Some(&project), &personal, 48 * 1024);
    let view = render(&settings(), &projection);
    assert!(
        view.contains(
            "No instruction sources found: looked for AGENTS.md in the workspace root and instructions.md in your config directory."
        ),
        "{view}"
    );
    assert!(view.contains("project instructions on"), "{view}");
    assert!(view.contains("personal instructions on"), "{view}");
    assert!(
        view.contains("limits: project 32768 B, personal 16384 B, total 49152 B."),
        "the three effective byte limits are shown: {view}"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn disabled_flags_are_shown_without_claiming_a_look() {
    let root = stamp("disabled");
    let personal = root.join("personal");
    let project = root.join("project");
    write_file(&personal.join("instructions.md"), "personal body\n");
    write_file(&project.join("AGENTS.md"), "project body\n");
    let mut off = settings();
    off.project_enabled = false;
    off.personal_enabled = false;
    let projection = project_instructions(&off, Some(&project), &personal, 48 * 1024);
    let view = render(&off, &projection);
    assert!(view.contains("project instructions off"), "{view}");
    assert!(view.contains("personal instructions off"), "{view}");
    assert!(view.contains("disabled"), "{view}");
    assert!(
        !view.contains("looked for"),
        "disabled sources are never claimed to have been read: {view}"
    );
    let _ = std::fs::remove_dir_all(root);
}
