//! One-reader drift guard for `[limits]`.
//!
//! Every surface's ceilings are resolved once in `saya-config`, and each
//! surface projects them into its engine through exactly one adapter. Before
//! `[limits]` existed, the session, `ask`, run, and coding surfaces each read
//! their own ceiling keys, and those readers drifted. This guard pins the
//! reader set: a module that reaches into `resolved.limits.*` outside the
//! adapter (or the config crate that defines, resolves, and displays the
//! table) fails here. Add a reader by moving it into an adapter — not by
//! widening this list.

use std::path::{Path, PathBuf};

/// The enforcement adapters: the only code outside `saya-config` allowed to
/// read a resolved ceiling. One per engine.
const ADAPTERS: &[&str] = &[
    // Session and `ask` -> `saya_agent::{AnsweringLimits, AnsweringScope}`.
    "crates/saya-cli/src/agent/limits.rs",
    // Coding root, helpers, and the durable ledger -> `CountLimits` and friends.
    "crates/saya-cli/src/commands/coding/limits.rs",
    // Run -> `saya_types::Budgets` through `ResolvedRunLimits::budgets`.
    "crates/saya-cli/src/commands/run/start/prepare.rs",
];

/// Display-only readers: `config doctor` and `config show` report ceilings;
/// they never feed one to an engine.
const DISPLAY: &[&str] = &[
    "crates/saya-cli/src/config/limits_report.rs",
    // `coding doctor` receives the resolved table and publishes it.
    "crates/saya-cli/src/commands/coding/doctor.rs",
    "crates/saya-cli/src/commands/coding/doctor_budget.rs",
];

/// Where the table is defined, resolved, protected, merged, and displayed.
/// These are the config crate's own concerns, not enforcement readers.
const CONFIG: &[&str] = &[
    "crates/saya-config/src/limits.rs",
    "crates/saya-config/src/limits",
    "crates/saya-config/src/diagnostics.rs",
    "crates/saya-config/src/diagnostics",
    "crates/saya-config/src/layers",
    // The field declaration and the crate's re-export.
    "crates/saya-config/src/resolve.rs",
    "crates/saya-config/src/lib.rs",
];

/// The four resolved sub-tables, as a dotted access.
const MARKERS: &[&str] = &[
    ".limits.session",
    ".limits.ask",
    ".limits.run",
    ".limits.job",
];

/// Reading a dimension without the dotted path still reads it: naming the
/// resolved type is how a display module receives the table.
const TYPE_MARKERS: &[&str] = &[
    "ResolvedAnsweringLimits",
    "ResolvedRunLimits",
    "ResolvedJobLimits",
    "ResolvedLimits",
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/saya-cli has a workspace root above it")
        .to_path_buf()
}

/// Every first-party source file under `crates/*/src`, excluding test modules:
/// the guard is about production readers, and a test may assert on any field.
fn source_files(root: &Path) -> Vec<PathBuf> {
    fn walk(directory: &Path, files: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, files);
            } else if path.extension().is_some_and(|ext| ext == "rs") && !is_test_module(&path) {
                files.push(path);
            }
        }
    }
    let mut files = Vec::new();
    for crate_dir in std::fs::read_dir(root.join("crates"))
        .expect("the workspace has a crates directory")
        .flatten()
    {
        walk(&crate_dir.path().join("src"), &mut files);
    }
    files
}

fn is_test_module(path: &Path) -> bool {
    path.components().any(|part| {
        let name = part.as_os_str().to_string_lossy();
        name == "tests" || name.ends_with("_tests") || name.ends_with("_tests.rs")
    })
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .expect("every source file is under the workspace root")
        .to_string_lossy()
        .into_owned()
}

fn allowed(relative: &str) -> bool {
    let prefix_match = |prefix: &str| {
        relative == prefix
            || relative
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with('/'))
    };
    ADAPTERS.contains(&relative)
        || DISPLAY.contains(&relative)
        || CONFIG.iter().any(|prefix| prefix_match(prefix))
}

#[test]
fn only_the_adapters_read_a_resolved_limit() {
    let root = workspace_root();
    // The adapter set must exist: a guard that allows a path nobody owns
    // would hide the missing adapter rather than catch the reader.
    for adapter in ADAPTERS.iter().chain(DISPLAY) {
        assert!(
            root.join(adapter).is_file(),
            "the limits guard names {adapter}, but it does not exist"
        );
    }
    let mut offenders = Vec::new();
    for path in source_files(&root) {
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("must read {}: {error}", path.display()));
        let relative = relative(&root, &path);
        if allowed(&relative) {
            continue;
        }
        for (index, line) in source.lines().enumerate() {
            // Comments may name the table; only code that reaches into it
            // counts.
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            if MARKERS.iter().any(|marker| line.contains(marker))
                || TYPE_MARKERS.iter().any(|marker| line.contains(marker))
            {
                offenders.push(format!("{relative}:{}: {}", index + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "resolved `[limits]` is read outside the adapters; move the read into \
         the surface's adapter:\n{}",
        offenders.join("\n")
    );
}
