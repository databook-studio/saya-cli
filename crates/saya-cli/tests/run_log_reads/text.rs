//! Guarantee 1: text mode renders the shaper's own line per event
//! (`crate::render_run` is the one shaper) — write order, one line per
//! recorded event.

use super::{
    lock_env, restore_runs_dir, run_log, runtime_at, seed_run, seeded_events, set_runs_dir,
    store_at, temp_root,
};
use saya_cli::RenderFormat;
use std::fs;

/// Text mode renders the shaper's own line per event, in write order, one
/// line per recorded event — including the usage event the engine journals
/// per provider call.
#[tokio::test]
async fn run_log_text_renders_the_journals_events_in_write_order() {
    let _env = lock_env().await;
    let root = temp_root("text");
    unsafe { set_runs_dir(&root.join("runs")) };
    let runtime = runtime_at(&root);
    let store = store_at(&root).await;
    let events = seeded_events();
    seed_run(&root, &store, "r-log-text", &events).await;

    let (code, out, err) = run_log("r-log-text", &runtime, &store, RenderFormat::Text).await;

    assert_eq!(code, 0, "stderr: {err}");
    let expected = [
        "run started",
        "plan approved",
        "step 1 started",
        "usage · primary · tokens 100 · cache reads unknown · cache writes unknown · turns unknown · tool calls unknown",
        "step 1 completed",
        "run completed",
    ]
    .join("\n");
    assert_eq!(
        out,
        format!("{expected}\n"),
        "the log is the journal, in write order, one line per event"
    );

    unsafe { restore_runs_dir(None) };
    let _ = fs::remove_dir_all(root);
}
