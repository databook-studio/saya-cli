//! Every relative link in `docs/` and in a crate's markdown resolves.
//!
//! A link to a file a deletion unit removed reads exactly like a live one, so
//! nothing but a walk of the tree catches it. This test is that walk: it
//! resolves each `[text](target)` beside the file it appears in, and fails on
//! the ones that name nothing.

use std::fs;
use std::path::{Path, PathBuf};

/// The workspace root: `crates/saya-cli` → up two.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate sits two levels under the workspace root")
        .to_path_buf()
}

fn markdown_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let skip = matches!(
            entry.file_name().to_str(),
            Some("target" | ".git" | "__pycache__")
        );
        if path.is_dir() {
            if !skip {
                markdown_files(&path, out);
            }
        } else if path.extension().is_some_and(|ext| ext == "md") {
            out.push(path);
        }
    }
}

/// The `target` of every `[text](target)` link in `text`, in order.
fn markdown_targets(text: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("](") {
        rest = &rest[start + 2..];
        let Some(end) = rest.find(')') else {
            break;
        };
        let target = rest[..end].trim();
        let target = target.split_whitespace().next().unwrap_or(target);
        targets.push(target.to_owned());
        rest = &rest[end..];
    }
    targets
}

/// Whether a link target is one this test does not resolve: an anchor, an
/// absolute URL, or empty.
fn external(target: &str) -> bool {
    target.is_empty()
        || target.starts_with('#')
        || target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with("mailto:")
        || target.starts_with('/')
}

#[test]
fn every_relative_documentation_link_resolves() {
    let root = workspace_root();
    let mut docs = Vec::new();
    markdown_files(&root.join("docs"), &mut docs);
    for crate_dir in fs::read_dir(root.join("crates"))
        .expect("the workspace has crates")
        .flatten()
    {
        markdown_files(&crate_dir.path(), &mut docs);
    }
    let mut broken = Vec::new();
    for doc in &docs {
        let text = fs::read_to_string(doc).unwrap_or_default();
        for target in markdown_targets(&text) {
            if external(&target) {
                continue;
            }
            let relative = target.split('#').next().unwrap_or(&target);
            if relative.is_empty() {
                continue;
            }
            let resolved = doc
                .parent()
                .expect("a markdown file has a parent")
                .join(relative);
            if !resolved.exists() {
                broken.push(format!(
                    "{} -> {target}",
                    doc.strip_prefix(&root).unwrap_or(doc).display()
                ));
            }
        }
    }
    broken.sort();
    assert!(
        broken.is_empty(),
        "{} broken documentation link(s): {broken:#?}",
        broken.len()
    );
}
