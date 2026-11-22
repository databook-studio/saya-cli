//! The old TUI's trunk is gone, and every leaf the new surface reuses stayed.
//!
//! X1 deletes the surface T19 stopped launching. Two things must not drift
//! after it: a trunk module creeping back (a copy of the replaced surface is
//! exactly what this deletion removed), and a kept leaf being deleted with it
//! — the new surface imports those through `interactive::tui`, so losing one
//! breaks the surface that shipped.
//!
//! The guard reads the tree, like `limits_single_reader` reads the sources: it
//! names paths rather than symbols, so it stays a statement about the layout
//! the brief asked for.

use std::path::{Path, PathBuf};

/// The old surface's replaced trunk: directories and files that only it used.
const TRUNK_GONE: &[&str] = &[
    // The state and plumbing of the replaced surface.
    "interactive/tui/types",
    "interactive/tui/application",
    "interactive/tui/agent",
    "interactive/tui/dispatch",
    "interactive/tui/loop_tick",
    "interactive/tui/transcript",
    "interactive/tui/transcript.rs",
    "interactive/tui/keys",
    "interactive/tui/stream_events",
    "interactive/tui/session",
    "interactive/tui/snapshots",
    // The old frame: chrome, the input box's painter, the overlays, and the
    // empty/splash/transcript panes the new `front/tui/view` paints instead.
    "interactive/tui/ui/chrome",
    "interactive/tui/ui/input",
    "interactive/tui/ui/overlays",
    "interactive/tui/ui/surface/empty_state",
    "interactive/tui/ui/surface/empty_state.rs",
    "interactive/tui/ui/surface/empty_state_tests",
    "interactive/tui/ui/surface/empty_state_tests.rs",
    "interactive/tui/ui/surface/splash.rs",
    "interactive/tui/ui/surface/splash_tests.rs",
    "interactive/tui/ui/surface/transcript.rs",
    "interactive/tui/ui/surface/transcript_draft_tests.rs",
    // The named file families: the app's helpers, its panels, its saves.
    "interactive/tui/atref.rs",
    "interactive/tui/capture_agent.rs",
    "interactive/tui/capture_agent_tests.rs",
    "interactive/tui/capture_agent_tests",
    "interactive/tui/capture_save_tests.rs",
    "interactive/tui/capture_save_tests",
    "interactive/tui/capture_save_sql_cancellation_tests.rs",
    "interactive/tui/continuation_acceptance_tests.rs",
    "interactive/tui/continuation_acceptance_tests",
    "interactive/tui/dispatch_actions.rs",
    "interactive/tui/dispatch_contracts.rs",
    "interactive/tui/dispatch_contracts_tests.rs",
    "interactive/tui/dispatch_investigation.rs",
    "interactive/tui/dispatch_investigation_tests.rs",
    "interactive/tui/dispatch_runs.rs",
    "interactive/tui/metrics_summary_tests.rs",
    "interactive/tui/replay_task.rs",
    "interactive/tui/replay_task_tests.rs",
    "interactive/tui/replay_task_tests",
    "interactive/tui/resume_capture_tests.rs",
    "interactive/tui/resume_capture_tests",
    "interactive/tui/run_panel.rs",
    "interactive/tui/run_panel_apply.rs",
    "interactive/tui/run_panel_snapshot_support.rs",
    "interactive/tui/run_panel_snapshot_tests.rs",
    "interactive/tui/run_panel_support.rs",
    "interactive/tui/run_panel_tests.rs",
    "interactive/tui/run_panel_worker.rs",
    "interactive/tui/run_worker.rs",
    "interactive/tui/scope_tests.rs",
    "interactive/tui/session_save.rs",
    "interactive/tui/session_save_tests.rs",
    "interactive/tui/setup_apply.rs",
    "interactive/tui/startup_readiness.rs",
    "interactive/tui/startup_readiness_tests.rs",
    "interactive/tui/truncation_tests.rs",
    "interactive/tui/typed_resume_capture_tests.rs",
    "interactive/tui/typed_resume_capture_tests",
    "interactive/tui/types_support.rs",
    "interactive/tui/types_tests.rs",
    "interactive/tui/usage_footer.rs",
    "interactive/tui/usage_totals.rs",
    "interactive/tui/ui_snapshot_approval_modal.rs",
    "interactive/tui/ui_snapshot_claims.rs",
    "interactive/tui/ui_snapshot_cursor_labels.rs",
    "interactive/tui/ui_snapshot_drafts.rs",
    "interactive/tui/ui_snapshot_fold_composer.rs",
    "interactive/tui/ui_snapshot_gate_action.rs",
    "interactive/tui/ui_snapshot_output_scope.rs",
    "interactive/tui/ui_snapshot_splash_wide.rs",
    "interactive/tui/ui_snapshot_support.rs",
    "interactive/tui/ui_snapshot_tables.rs",
    "interactive/tui/ui_snapshot_tests.rs",
];

/// The leaves the new surface imports through `interactive::tui` — deleting
/// one breaks `front/tui`, so each must still name a file or a directory.
const KEPT_LEAVES: &[&str] = &[
    "interactive/tui/input",
    "interactive/tui/wrap.rs",
    "interactive/tui/ui/surface/markdown.rs",
    "interactive/tui/ui/surface/starter.rs",
    "interactive/tui/ui/surface/starter_tests.rs",
    "interactive/tui/table",
    "interactive/tui/ui/theme/detect.rs",
    "interactive/tui/ui/theme/detect_tests.rs",
    "interactive/tui/ui/theme/hue.rs",
    "interactive/tui/ui/theme/palette.rs",
    "interactive/tui/clipboard.rs",
    "interactive/tui/fuzzy.rs",
    "interactive/tui/history",
    "interactive/tui/terminal.rs",
    "interactive/tui/terminal_handoff.rs",
    "interactive/tui/sql_task",
    "interactive/tui/sql_task.rs",
    "interactive/tui/exec.rs",
    "interactive/tui/export",
    "interactive/tui/capture.rs",
    "interactive/tui/trust.rs",
    "interactive/tui/complete",
    "interactive/tui/coding",
    "interactive/tui/worker_permits.rs",
    "interactive/tui/worker_permits_tests.rs",
    "interactive/tui/replay.rs",
    // The session's one token accumulator, kept where the line front reads it.
    "interactive/usage/mod.rs",
    // The two helpers `/chart` and `/investigation save` moved to the surface
    // that calls them, rather than dying with the dispatcher they rode.
    "front/tui/direct_save.rs",
    "front/tui/direct_chart_args.rs",
];

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn exists(relative: &str) -> bool {
    crate_root().join("src").join(relative).exists()
}

/// Each entry absent is one assertion met; the failure names the ones that
/// came back.
#[test]
fn the_old_surface_trunk_is_gone() {
    let present: Vec<&str> = TRUNK_GONE
        .iter()
        .copied()
        .filter(|path| exists(path))
        .collect();
    assert!(
        present.is_empty(),
        "the old TUI trunk is still on disk ({} path(s)); X1 deletes it:\n{}",
        present.len(),
        present.join("\n")
    );
}

#[test]
fn every_leaf_the_new_surface_reuses_survives() {
    let missing: Vec<&str> = KEPT_LEAVES
        .iter()
        .copied()
        .filter(|path| !exists(path))
        .collect();
    assert!(
        missing.is_empty(),
        "the new surface imports these through `interactive::tui`, but they are \
         gone ({} path(s)):\n{}",
        missing.len(),
        missing.join("\n")
    );
}

/// `interactive::tui` carried a module-level `#[allow(dead_code)]` while the
/// old surface waited to be deleted: the whole subtree was unreachable from
/// the binary. With the trunk gone, nothing dead may remain, so the allow
/// must not come back.
#[test]
fn the_old_surface_carries_no_dead_code_allowance() {
    let source = std::fs::read_to_string(crate_root().join("src/interactive/mod.rs"))
        .expect("read the interactive module");
    let offenders: Vec<&str> = source
        .lines()
        .enumerate()
        .filter(|(_, line)| line.trim() == "#[allow(dead_code)]")
        .filter(|(index, _)| {
            // The allowance sits directly above the surface declaration, as
            // every other module-level allowance here does.
            source
                .lines()
                .nth(index + 1)
                .is_some_and(|next| next.contains("mod tui;"))
        })
        .map(|(_, line)| line.trim())
        .collect();
    assert!(
        offenders.is_empty(),
        "`interactive::tui` still carries a dead-code allowance; the trunk it \
         covered is deleted, so the allowance hides whatever is left: {offenders:?}"
    );
}

/// Nothing launches or names the old entry point any more (T19 switched the
/// launch to `front/tui`). The needles are spelled in halves so this guard's
/// own source never matches them.
#[test]
fn the_old_launcher_is_named_nowhere() {
    let needles = [
        concat!("Tui", "Session"),
        concat!("interactive::tui::", "run"),
    ];
    let mut offenders = Vec::new();
    for path in rust_sources(&crate_root()) {
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("must read {}: {error}", path.display()));
        for (index, line) in source.lines().enumerate() {
            if needles.iter().any(|needle| line.contains(needle)) {
                offenders.push(format!("{}:{}: {}", path.display(), index + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "the old TUI launcher is named in {} place(s):\n{}",
        offenders.len(),
        offenders.join("\n")
    );
}

/// Every first-party source under this crate, tests included: the launch is a
/// call site too.
fn rust_sources(root: &Path) -> Vec<PathBuf> {
    fn walk(directory: &Path, files: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, files);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                files.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(&root.join("src"), &mut files);
    walk(&root.join("tests"), &mut files);
    files
}
