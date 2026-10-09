//! Guarantee 3: a run whose journal holds no events says so, rather than
//! succeeding with empty output that reads as "nothing happened is nothing".

use super::{
    lock_env, restore_runs_dir, run_log, runtime_at, seed_run, set_runs_dir, store_at, temp_root,
};
use saya_cli::RenderFormat;
use std::fs;

/// A run with no journal events renders something honest rather than an
/// empty success that reads as "the journal was empty on purpose".
#[tokio::test]
async fn a_run_with_no_journal_events_renders_something_honest() {
    let _env = lock_env().await;
    let root = temp_root("empty");
    unsafe { set_runs_dir(&root.join("runs")) };
    let runtime = runtime_at(&root);
    let store = store_at(&root).await;
    seed_run(&root, &store, "r-log-empty", &[]).await;

    let (code, out, err) = run_log("r-log-empty", &runtime, &store, RenderFormat::Text).await;

    assert_eq!(code, 0, "stderr: {err}");
    assert!(
        out.contains("no journal events"),
        "the honest message, not empty output: {out:?}"
    );

    unsafe { restore_runs_dir(None) };
    let _ = fs::remove_dir_all(root);
}
