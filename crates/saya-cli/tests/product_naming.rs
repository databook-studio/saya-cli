//! Saya's shipped surface names Saya, not other coding-agent products or a
//! private evaluation suite. This guard scans the committed text a reader or a model
//! can see: crate sources and fixtures, docs, the changelog and release notes,
//! the agent skills, scripts and examples.
//!
//! Provider and model identifiers are not harness names and are not scanned.
//! Dot-directories such as `.codex/` are configuration paths a privacy policy
//! must exclude, not product naming; the scanner strips that path form before
//! matching.

use std::path::{Path, PathBuf};

/// Product and evaluation-suite names that must not appear in the product, stored
/// reversed so this guard's own source never matches the needles it scans for.
const BANNED_REVERSED: &[&str] = &[
    "edoC edualC",
    "edoc-edualc",
    "edoC nepO",
    "xedoC",
    "rediA",
    "enilC",
    "tnega-EWS",
    "dlofkcotS",
];

fn forward(reversed: &str) -> String {
    reversed.chars().rev().collect()
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn scanned_paths(root: &Path) -> Vec<PathBuf> {
    let mut paths = vec![
        root.join("crates"),
        root.join("docs"),
        root.join(".agents"),
        root.join("scripts"),
        root.join("examples"),
        root.join("README.md"),
        root.join("CHANGELOG.md"),
        root.join("RELEASING.md"),
        root.join("HOST-COMMANDS-v1.md"),
    ];
    paths.retain(|path| path.exists());
    paths
}

fn collect_text_files(path: &Path, found: &mut Vec<PathBuf>) {
    if path.is_dir() {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            collect_text_files(&entry.path(), found);
        }
        return;
    }
    let text = matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("md" | "rs" | "toml" | "json" | "txt" | "sh" | "py" | "yml" | "yaml")
    );
    if text {
        found.push(path.to_path_buf());
    }
}

/// Lowercase alphanumeric words joined by single spaces, so spacing,
/// punctuation and line breaks do not change a comparison, while a longer word
/// never equals a shorter one.
fn normalize(text: &str) -> String {
    let mut out = String::new();
    let mut pending_space = false;
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.push(ch.to_ascii_lowercase());
        } else {
            pending_space = true;
        }
    }
    out
}

fn mentions(text: &str, needle: &str) -> bool {
    let text = normalize(&text.replace(".codex", ""));
    format!(" {text} ").contains(&format!(" {} ", normalize(needle)))
}

#[test]
fn the_product_does_not_name_other_coding_agents_or_a_private_evaluation_suite() {
    let root = repo_root();
    let mut files = Vec::new();
    for path in scanned_paths(&root) {
        collect_text_files(&path, &mut files);
    }
    // This guard is the only scanned file allowed to discuss the names at all.
    files.retain(|file| {
        file.file_name()
            .is_none_or(|name| name != "product_naming.rs")
    });

    let mut offenders = Vec::new();
    for file in files {
        let Ok(contents) = std::fs::read_to_string(&file) else {
            continue;
        };
        for reversed in BANNED_REVERSED {
            let needle = forward(reversed);
            if mentions(&contents, &needle) {
                offenders.push(format!(
                    "{}: {}",
                    file.strip_prefix(&root).unwrap_or(&file).display(),
                    needle
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "the product may not name another coding-agent product or a private \
         evaluation suite; found:\n{}",
        offenders.join("\n")
    );
}
