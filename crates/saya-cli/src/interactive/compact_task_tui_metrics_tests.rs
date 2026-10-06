//! The TUI compaction worker delivers the session's collector to the shared
//! compaction operation: when the App carries the runtime-only collector
//! (some `--verbose` launches), each worker run records its Compaction phase
//! there — one Succeeded or Failed sample plus CompactionStarted, exactly
//! like the headless recording. A collector-less app behaves as before.

use super::compact_task;
use super::session_state::SessionState;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::tui::types::App;
use crate::interactive::tui::types::app::idle_app;
use crate::local_metrics::{
    LocalMetrics, LocalPhaseSnapshot, PhaseCell, count_index, outcome_index, phase_index,
};
use saya_agent::{AgentMetricCount, AgentMetricOutcome, AgentMetricPhase};
use saya_config::{
    AiProvider, ColorChoice, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs, ResolvedJobs, ResolvedMemory,
    ResolvedRunnerJobs, ThemeChoice,
};
use std::{
    io::{Read, Write as _},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};
/// A local HTTP source counting every request it receives.
fn counting_source(
    seen: Arc<Mutex<usize>>,
    release: mpsc::Receiver<()>,
) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind preflight source");
    listener.set_nonblocking(true).expect("source nonblocking");
    let address = listener.local_addr().expect("source address");
    let handle = thread::spawn(move || {
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    if read_request_body(&mut stream).is_ok() {
                        *seen.lock().expect("request count lock") += 1;
                        let _ = respond(&mut stream, ANSWER_RESPONSE);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if release.try_recv().is_ok() {
                        return;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return,
            }
        }
    });
    (format!("http://{address}/v1"), handle)
}
fn read_request_body(stream: &mut TcpStream) -> Result<String, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let (header_end, body_len) = loop {
        let read = stream
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("source request ended before headers".into());
        }
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let header = std::str::from_utf8(&bytes[..end]).map_err(|error| error.to_string())?;
            let len = header
                .lines()
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .map(|(_, value)| value.trim())
                })
                .ok_or_else(|| "source request omitted content length".to_string())?
                .parse::<usize>()
                .map_err(|error| error.to_string())?;
            break (end + 4, len);
        }
    };
    while bytes.len() < header_end + body_len {
        let read = stream
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("source request ended before body".into());
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    String::from_utf8(bytes[header_end..header_end + body_len].to_vec())
        .map_err(|error| error.to_string())
}

fn respond(stream: &mut TcpStream, body: &str) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes())
}
const ANSWER_RESPONSE: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"done.\"},\"finish_reason\":\"stop\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":1}}\n\ndata: [DONE]\n\n";
fn turns(count: usize) -> SessionState {
    let mut state = SessionState::new("s1", None, "m");
    for index in 0..count {
        state.record_turn(format!("q{index}"), format!("a{index}"), false, Vec::new());
    }
    state
}

/// The same turns, pointed at the OpenaiCompatible provider so the state's
/// per-prompt override keeps the runtime's source-bound `base_url` intact.
fn openai_turns(count: usize) -> SessionState {
    let mut state = turns(count);
    state.provider = "openai_compatible".into();
    state
}
fn test_runtime() -> crate::config::runtime::RuntimeConfig {
    crate::config::runtime::RuntimeConfig {
        resolved: ResolvedConfig {
            instructions: Default::default(),
            profile_name: None,
            profile: None,
            investigation_budgets: Default::default(),
            ai: ResolvedAi {
                provider: AiProvider::Ollama,
                model: "test-model".into(),
                base_url: None,
                api_key: None,
                allow_data_sharing: true,
                temperature: 0.0,
                timeout_seconds: 60,
                idle_timeout_seconds: 90,
                max_output_tokens: 4096,
                max_output_tokens_is_default: true,
                context_byte_budget: 256 * 1024,
                context_window_tokens: None,
                show_thinking: false,
                compaction: saya_config::CompactionMode::Auto,
                retry_delays_ms: vec![250, 500, 1000],
            },
            max_rows: 100,
            read_only: true,
            max_iterations: 4,
            candidates: 1,
            jobs: ResolvedJobs {
                wall_clock_seconds: None,
                tokens_per_endpoint: Default::default(),
                turns: Some(4),
                tool_calls: None,
                fetch: ResolvedFetchJobs::default(),
                interpreter: ResolvedInterpreterJobs::default(),
                runner: ResolvedRunnerJobs::default(),
            },
            query_timeout_seconds: 5,
            output_format: OutputFormat::Text,
            output_color: ColorChoice::Auto,
            ui_theme: ThemeChoice::Auto,
            memory: ResolvedMemory {
                mode: MemoryMode::Off,
                max_contracts: 5,
                max_claims_per_contract: 12,
                max_context_bytes: 16384,
            },
            host_commands: ResolvedHostCommands::default(),
            session_deny: Default::default(),
            ignored_project_overrides: Vec::new(),
            provenance: Default::default(),
            endpoints: Default::default(),
        },
        connections: Default::default(),
        config_path: None,
        connections_path: None,
        cache_scope: PathBuf::from("/tmp/saya-session-question"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: Default::default(),
    }
}

/// A runtime whose provider points at `base_url`: the compaction call runs
/// the same OpenaiCompatible shape the preflight fixtures dispatch through.
fn runtime_for_source(base_url: String, context_window_tokens: Option<u64>) -> RuntimeConfig {
    let mut runtime = test_runtime();
    runtime.resolved.ai = ResolvedAi {
        provider: AiProvider::OpenaiCompatible,
        model: "metrics-test".into(),
        base_url: Some(base_url),
        api_key: None,
        allow_data_sharing: true,
        temperature: 0.0,
        timeout_seconds: 5,
        idle_timeout_seconds: 5,
        max_output_tokens: 32,
        max_output_tokens_is_default: false,
        context_byte_budget: 256 * 1024,
        context_window_tokens,
        show_thinking: false,
        compaction: saya_config::CompactionMode::Auto,
        retry_delays_ms: Vec::new(),
    };
    runtime
}
fn compaction_cell(snapshot: &LocalPhaseSnapshot) -> PhaseCell {
    let index = phase_index(AgentMetricPhase::Compaction).expect("compaction has a slot");
    snapshot.phases[index]
}

fn started_count(snapshot: &LocalPhaseSnapshot) -> u64 {
    let index = count_index(AgentMetricCount::CompactionStarted).expect("started has a slot");
    snapshot.counts[index]
}
/// Waits for the worker the way the event loop does: poll each tick until
/// the task clears, bounded at 20 s with 20 ms sleeps.
fn wait_for_worker(app: &mut App, state: &mut SessionState) {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while app.compact_task.is_some() {
        assert!(
            std::time::Instant::now() < deadline,
            "the worker never finished within 20s"
        );
        compact_task::poll(app, state);
        thread::sleep(Duration::from_millis(20));
    }
}
#[test]
fn the_tui_worker_records_the_session_collectors_succeeded_sample() {
    let mut app = idle_app();
    app.session_metrics = Some(Arc::new(LocalMetrics::new()));
    let seen = Arc::new(Mutex::new(0_usize));
    let (release_tx, release_rx) = mpsc::channel();
    let (base_url, source) = counting_source(Arc::clone(&seen), release_rx);
    app.runtime = Arc::new(runtime_for_source(base_url, Some(64)));
    let mut state = openai_turns(4);
    compact_task::start(&mut app, &state);
    wait_for_worker(&mut app, &mut state);
    release_tx.send(()).expect("release source");
    source.join().expect("source thread joins");
    let snapshot = app.session_metrics.expect("the collector rides").snapshot();
    let cell = compaction_cell(&snapshot);
    assert_eq!(cell.samples, 1, "one compaction sample: {snapshot:?}");
    let ok = outcome_index(AgentMetricOutcome::Succeeded).expect("succeeded has a slot");
    assert_eq!(
        cell.outcomes[ok], 1,
        "the sample is Succeeded: {snapshot:?}"
    );
    assert_eq!(
        started_count(&snapshot),
        1,
        "the summariser call started once"
    );
    assert!(
        app.transcript
            .blocks()
            .iter()
            .any(|block| block.text.contains("turns into a summary")),
        "the transcript announces the compaction"
    );
}
#[test]
fn a_dead_provider_records_one_failed_sample_through_the_tui_worker() {
    let mut app = idle_app();
    app.session_metrics = Some(Arc::new(LocalMetrics::new()));
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a port");
    let address = listener.local_addr().expect("the closed port's address");
    drop(listener);
    app.runtime = Arc::new(runtime_for_source(format!("http://{address}/v1"), Some(64)));
    let mut state = openai_turns(4);
    compact_task::start(&mut app, &state);
    wait_for_worker(&mut app, &mut state);
    let snapshot = app.session_metrics.expect("the collector rides").snapshot();
    let cell = compaction_cell(&snapshot);
    assert_eq!(cell.samples, 1, "one compaction sample: {snapshot:?}");
    let failed = outcome_index(AgentMetricOutcome::Failed).expect("failed has a slot");
    assert_eq!(
        cell.outcomes[failed], 1,
        "the sample is Failed: {snapshot:?}"
    );
}
#[test]
fn a_collector_less_tui_worker_matches_the_recorded_worker() {
    let seen = Arc::new(Mutex::new(0_usize));
    let (release_tx, release_rx) = mpsc::channel();
    let (base_url, source) = counting_source(Arc::clone(&seen), release_rx);
    let runtime = Arc::new(runtime_for_source(base_url, Some(64)));
    let mut recorded = idle_app();
    recorded.session_metrics = Some(Arc::new(LocalMetrics::new()));
    recorded.runtime = Arc::clone(&runtime);
    let mut bare = idle_app();
    bare.runtime = Arc::clone(&runtime);
    let mut recorded_state = openai_turns(4);
    let mut bare_state = openai_turns(4);
    compact_task::start(&mut recorded, &recorded_state);
    compact_task::start(&mut bare, &bare_state);
    wait_for_worker(&mut recorded, &mut recorded_state);
    wait_for_worker(&mut bare, &mut bare_state);
    release_tx.send(()).expect("release source");
    source.join().expect("source thread joins");
    let blocks = |app: &App| {
        app.transcript
            .blocks()
            .iter()
            .map(|block| block.text.to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        blocks(&recorded),
        blocks(&bare),
        "the transcript is identical"
    );
    assert_eq!(
        recorded_state.provider_history(),
        bare_state.provider_history(),
        "the live conversation is identical"
    );
    assert_eq!(
        *seen.lock().expect("request count lock"),
        2,
        "each worker called once"
    );
}
