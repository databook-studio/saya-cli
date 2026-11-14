//! Guarantee 2: JSON and NDJSON keep the journal's own bytes — the one serde
//! path the live wire streams — so a script parsing `--format ndjson` parses
//! the durable record, not a re-rendering of it.

use super::{
    lock_env, restore_runs_dir, run_log, runtime_at, seed_run, seeded_events, set_runs_dir,
    store_at, temp_root,
};
use saya_cli::RenderFormat;
use std::fs;

/// JSON and NDJSON keep the journal's own bytes: the message carries the
/// events' one serde path, in write order — the framing machine consumers parse.
#[tokio::test]
async fn run_log_ndjson_carries_the_journals_own_lines() {
    let _env = lock_env().await;
    let root = temp_root("ndjson");
    unsafe { set_runs_dir(&root.join("runs")) };
    let runtime = runtime_at(&root);
    let store = store_at(&root).await;
    let events = seeded_events();
    seed_run(&root, &store, "r-log-wire", &events).await;

    let (code, out, err) = run_log("r-log-wire", &runtime, &store, RenderFormat::Ndjson).await;

    assert_eq!(code, 0, "stderr: {err}");
    let envelope: serde_json::Value = serde_json::from_str(out.trim_end()).unwrap();
    let message = envelope["message"].as_str().expect("result envelope");
    let expected = events
        .iter()
        .map(|event| serde_json::to_string(event).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        message, expected,
        "the NDJSON framing is the journal's own serialization, in write order"
    );

    unsafe { restore_runs_dir(None) };
    let _ = fs::remove_dir_all(root);
}
