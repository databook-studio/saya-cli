//! Every `saya …` command line a reference page shows in a shell block must
//! parse against the real clap surface. The pages are prose, so lines with
//! placeholders (`<id>`), pipes, shell variables, or elisions are skipped —
//! they are not a command a reader can run as written. A stale subcommand or
//! flag in a runnable example fails here.

use clap::Parser;
use clap::error::ErrorKind;
use saya_cli::Cli;

/// The reference pages whose shell blocks are executable claims.
const PAGES: &[(&str, &str)] = &[
    (
        "docs/commands.md",
        include_str!("../../../../docs/commands.md"),
    ),
    (
        "docs/configuration.md",
        include_str!("../../../../docs/configuration.md"),
    ),
    (
        "docs/coding-capabilities.md",
        include_str!("../../../../docs/coding-capabilities.md"),
    ),
    (
        "docs/installation.md",
        include_str!("../../../../docs/installation.md"),
    ),
    (
        "docs/providers.md",
        include_str!("../../../../docs/providers.md"),
    ),
    (
        "docs/connections.md",
        include_str!("../../../../docs/connections.md"),
    ),
    (
        "docs/querying-databases.md",
        include_str!("../../../../docs/querying-databases.md"),
    ),
];

/// Split one shell word sequence, honouring single and double quotes.
fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for ch in line.chars() {
        match (quote, ch) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => current.push(c),
            (None, '\'' | '"') => quote = Some(ch),
            (None, c) if c.is_whitespace() => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            (None, c) => current.push(c),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// The runnable `saya` lines in the page's ```bash / ```sh blocks, with
/// backslash continuations joined and trailing comments removed.
fn runnable_lines(page: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut in_shell = false;
    let mut pending = String::new();
    for raw in page.lines() {
        let trimmed = raw.trim();
        if let Some(fence) = trimmed.strip_prefix("```") {
            in_shell = !in_shell && matches!(fence, "bash" | "sh");
            continue;
        }
        if !in_shell {
            continue;
        }
        let mut line = trimmed.to_string();
        if pending.is_empty() && !line.starts_with("saya ") {
            continue;
        }
        if let Some((before, _)) = line.split_once(" #") {
            line.truncate(before.len());
        }
        let continued = line.ends_with('\\');
        if continued {
            line.pop();
            line.push(' ');
            pending.push_str(line.trim_end());
            pending.push(' ');
            continue;
        }
        pending.push_str(line.trim());
        lines.push(std::mem::take(&mut pending));
    }
    lines
}

fn is_runnable(line: &str) -> bool {
    !line.contains('<')
        && !line.contains('>')
        && !line.contains("...")
        && !line.contains('|')
        && !line.contains('$')
        && !line.contains('\n')
}

#[test]
fn every_runnable_documented_command_parses() {
    let mut checked = 0;
    let mut failures = Vec::new();
    for (page, body) in PAGES {
        for line in runnable_lines(body) {
            if !is_runnable(&line) {
                continue;
            }
            checked += 1;
            let argv = words(&line);
            match Cli::try_parse_from(&argv) {
                Ok(_) => {}
                // `--version` and `--help` are successful parses that clap
                // reports as display errors.
                Err(error)
                    if matches!(
                        error.kind(),
                        ErrorKind::DisplayVersion | ErrorKind::DisplayHelp
                    ) => {}
                Err(error) => failures.push(format!("{page}: `{line}`\n    {error}")),
            }
        }
    }
    assert!(
        failures.is_empty(),
        "documented commands that no longer parse:\n{}",
        failures.join("\n")
    );
    assert!(
        checked >= 20,
        "the scan must actually cover the reference pages; checked {checked}"
    );
}
