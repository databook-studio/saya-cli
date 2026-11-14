//! Guard: a retired command spelling survives only where it is the answer —
//! the migration modules that name it and point at the spelling that works.
//! Every other non-test, user-visible line names the live command.

use std::path::{Path, PathBuf};

/// Files whose whole job is to name a retired spelling in its replacement.
const MIGRATION_FILES: &[&str] = &["cli/retired.rs", "slash/dispatch.rs"];

/// The spellings this rename retired.
const RETIRED_SPELLINGS: &[&str] = &["saya coding", "[coding]", "coding setup"];

/// A job may be code, documentation, analysis, or a question. Every
/// user- and model-facing line says "job"; these phrases assume code.
const CODING_PHRASES: &[&str] = &[
    "coding job",
    "coding jobs",
    "coding task",
    "coding operation",
    "coding session",
    "coding calls",
];

#[test]
fn no_user_visible_string_names_a_retired_command() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect(&root, &mut files);
    let mut offenders = Vec::new();
    for file in files {
        let relative = file
            .strip_prefix(&root)
            .expect("a source file below src")
            .to_string_lossy()
            .replace('\\', "/");
        if MIGRATION_FILES.contains(&relative.as_str()) || is_test(&relative) {
            continue;
        }
        let source = std::fs::read_to_string(&file).expect("a readable source file");
        for (index, line) in source.lines().enumerate() {
            // A comment is not shown to a user or a model.
            if line.trim_start().starts_with("//") {
                continue;
            }
            if names_retired(line) {
                offenders.push(format!("{relative}:{}: {}", index + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "retired command spellings in user-visible source:\n{}",
        offenders.join("\n")
    );
}

/// A job is not necessarily code, so no user- or model-facing text narrows it
/// to coding. Internal identifiers, paths, and comments are not text a user
/// reads; docs are all prose pages, and the changelog is checked only where
/// the next release is written.
#[test]
fn no_user_visible_text_calls_a_job_a_coding_one() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();

    let repository = root.join("../..");
    let mut sources = Vec::new();
    for entry in std::fs::read_dir(repository.join("crates")).expect("the workspace crates") {
        let path = entry.expect("a readable entry").path();
        if path.is_dir() && path.join("src").is_dir() {
            collect(&path.join("src"), &mut sources);
        }
    }
    for file in sources {
        let relative = file
            .strip_prefix(&repository)
            .expect("a source file below the repository root")
            .to_string_lossy()
            .replace('\\', "/");
        if is_test(&relative) {
            continue;
        }
        let source = std::fs::read_to_string(&file).expect("a readable source file");
        for (index, line) in source.lines().enumerate() {
            // A comment is not shown to a user or a model.
            if line.trim_start().starts_with("//") {
                continue;
            }
            if let Some(phrase) = names_coding_phrase(line) {
                offenders.push(format!("{relative}:{}: {phrase}", index + 1));
            }
        }
    }

    let docs = root.join("../../docs");
    let mut pages = Vec::new();
    collect_markdown(&docs, &mut pages);
    for file in pages {
        let relative = file
            .strip_prefix(root.join("../.."))
            .expect("a docs page below the repository root")
            .to_string_lossy()
            .replace('\\', "/");
        let source = std::fs::read_to_string(&file).expect("a readable docs page");
        for (index, line) in source.lines().enumerate() {
            if let Some(phrase) = names_coding_phrase(line) {
                offenders.push(format!("{relative}:{}: {phrase}", index + 1));
            }
        }
    }

    let readme = std::fs::read_to_string(root.join("../../README.md")).expect("README");
    for (index, line) in readme.lines().enumerate() {
        if let Some(phrase) = names_coding_phrase(line) {
            offenders.push(format!("README.md:{}: {phrase}", index + 1));
        }
    }
    let changelog = std::fs::read_to_string(root.join("../../CHANGELOG.md")).expect("CHANGELOG");
    for (number, line) in unreleased(&changelog) {
        if let Some(phrase) = names_coding_phrase(line) {
            offenders.push(format!("CHANGELOG.md:{number}: {phrase}"));
        }
    }

    assert!(
        offenders.is_empty(),
        "coding-specific phrases in user-visible text:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_job_prompt_does_not_call_itself_a_coding_agent() {
    let read_only = super::prompts::system(false, true);
    for (name, prompt) in [
        ("DIRECT", super::prompts::DIRECT),
        ("READ_ONLY", read_only.as_str()),
    ] {
        assert!(
            !prompt.contains("coding agent"),
            "{name} must address a generic job: {prompt}"
        );
    }
}

fn collect(directory: &Path, files: &mut Vec<PathBuf>) {
    collect_extension(directory, "rs", files);
}

/// Prose pages only: recordings and other media are not documentation text.
fn collect_markdown(directory: &Path, files: &mut Vec<PathBuf>) {
    collect_extension(directory, "md", files);
}

fn collect_extension(directory: &Path, extension: &str, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(directory).expect("a readable directory") {
        let path = entry.expect("a readable entry").path();
        if path.is_dir() {
            collect_extension(&path, extension, files);
        } else if path.extension().is_some_and(|found| found == extension) {
            files.push(path);
        }
    }
}

/// The lines of the `## Unreleased` section with their file line numbers.
fn unreleased(changelog: &str) -> Vec<(usize, &str)> {
    let mut lines = Vec::new();
    let mut inside = false;
    for (index, line) in changelog.lines().enumerate() {
        if line.starts_with("## ") {
            inside = line == "## Unreleased";
            continue;
        }
        if inside {
            lines.push((index + 1, line));
        }
    }
    lines
}

/// The first coding-specific phrase on a line, if any. Case-insensitive: a
/// heading capitalises the same phrase a sentence does.
fn names_coding_phrase(line: &str) -> Option<&'static str> {
    let lowered = line.to_ascii_lowercase();
    CODING_PHRASES
        .iter()
        .find(|phrase| lowered.contains(*phrase))
        .copied()
}

/// Test modules and test support are not user-visible text.
fn is_test(relative: &str) -> bool {
    relative
        .split('/')
        .any(|part| part.to_ascii_lowercase().contains("test"))
}

fn names_retired(line: &str) -> bool {
    if RETIRED_SPELLINGS
        .iter()
        .any(|spelling| line.contains(spelling))
    {
        return true;
    }
    // `/coding` is retired, but `commands/coding/` is a module path.
    let mut rest = line;
    while let Some(index) = rest.find("/coding") {
        let after = rest[index + "/coding".len()..].chars().next();
        if !after.is_some_and(|next| next.is_alphanumeric() || next == '_' || next == '/') {
            return true;
        }
        rest = &rest[index + 1..];
    }
    false
}
