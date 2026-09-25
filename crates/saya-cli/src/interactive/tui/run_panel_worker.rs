/// The real run worker end to end against an unreachable provider.
/// Moved byte-identical from the hub; no snapshots involved.
use super::super::run_panel::RunPanel;
use crate::interactive::tui::application::tests_support::idle_app;
use saya_agent::ApprovalPolicy;
use saya_store::SqliteStateStore;
use saya_types::RunEvent;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The real worker end to end against an unreachable provider.
/// Moved byte-identical from the hub; no snapshots involved.
/// The real worker, end to end, against a provider that cannot be reached:
/// the run claims its directory, the event loop keeps polling promptly while
/// the worker fails, and the panel ends with the run surface's own words.
#[tokio::test]
async fn a_panel_run_that_cannot_reach_its_provider_fails_into_the_panel() {
    let _env = crate::commands::RUNS_DIR.lock().await;
    let root = std::env::temp_dir().join(format!(
        "saya-run-panel-worker-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join("runs")).unwrap();
    let previous = std::env::var_os("SAYA_RUNS_DIR");
    // SAFETY: the runs-dir lock above serializes every unit test that points
    // the process-global variable at a private root.
    unsafe { std::env::set_var("SAYA_RUNS_DIR", root.join("runs")) };

    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let mut config = crate::interactive::tui::application::tests_support::unused_runtime();
    // An orchestrator endpoint is resolved but unreachable: the plan proposal
    // fails at the provider layer, which is the failure the panel must show.
    config.resolved.endpoints.insert(
        saya_config::ORCHESTRATOR_ROLE.to_string(),
        saya_config::ResolvedEndpoint {
            name: saya_config::ORCHESTRATOR_ROLE.to_string(),
            provider: saya_config::AiProvider::Ollama,
            model: "test-model".into(),
            base_url: Some("http://127.0.0.1:9/".into()),
            api_key: None,
        },
    );
    let runtime = Arc::new(config);
    let run_id = crate::commands::new_run_id();
    let worker = super::super::run_worker::spawn(super::super::run_worker::RunJob {
        runtime,
        state_db: store,
        format: crate::RenderFormat::Text,
        approval: ApprovalPolicy::ReadOnly,
        profile: None,
        request: crate::commands::RunRequest {
            run_id: run_id.clone(),
            goal: Some("survey the data".into()),
            allow: vec!["none".into()],
            budget: Vec::new(),
        },
    });
    let mut app = idle_app();
    app.run_panel = Some(RunPanel::new(
        worker,
        run_id.as_str().to_string(),
        "survey the data".into(),
    ));

    // The event loop's posture, exercised for real: every poll is prompt,
    // however long the worker takes, until the run settles.
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let tick = Instant::now();
        app.poll_run_panel(false);
        assert!(
            tick.elapsed() < Duration::from_millis(250),
            "the per-tick poll must never block on the run (took {:?})",
            tick.elapsed()
        );
        if !app.run_panel.as_ref().unwrap().is_active() {
            break;
        }
        assert!(Instant::now() < deadline, "the run never settled");
        std::thread::sleep(Duration::from_millis(50));
    }
    let panel = app.run_panel.as_ref().unwrap();
    assert!(
        panel.status.contains("could not bind a plan"),
        "the run surface's own words reached the panel: {:?}",
        panel.status
    );
    assert!(panel.status_is_error, "a failed run reads as an error");
    // The claim was real: the run's journal holds its first event, under the
    // runs root the panel's worker resolved.
    let journal = saya_harness::journal::Journal::open(root.join("runs").join(run_id.as_str()));
    let events = journal.read().unwrap();
    assert_eq!(events.first(), Some(&RunEvent::RunStarted));

    match previous {
        Some(value) => {
            // SAFETY: same serialized scope as above.
            unsafe { std::env::set_var("SAYA_RUNS_DIR", value) };
        }
        None => {
            // SAFETY: same serialized scope as above.
            unsafe { std::env::remove_var("SAYA_RUNS_DIR") };
        }
    }
    let _ = std::fs::remove_dir_all(root);
}
