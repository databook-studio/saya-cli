//! The session collector's Compaction phase, recorded by the shared
//! compaction operation: one sample per `/compact` invocation — Succeeded
//! when a summary validated and applied, Failed when the provider or
//! validation refused, Skipped for a no-work result — with
//! `CompactionStarted` counted only when the summariser call actually
//! starts. A collector-less run is otherwise identical.

use super::super::session_state::SessionState;
use super::super::session_universe::SessionUniverse;
use super::run_with_trigger;
use crate::config::runtime::RuntimeConfig;
use crate::local_metrics::{
    LocalMetrics, LocalPhaseSnapshot, PhaseCell, count_index, outcome_index, phase_index,
};
use saya_agent::{AgentMetricCount, AgentMetricOutcome, AgentMetricPhase};
use saya_config::{
    AiProvider, ColorChoice, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs, ResolvedJobs, ResolvedMemory,
    ResolvedRunnerJobs, ThemeChoice,
};
use saya_store::StoredContinuation;
use saya_types::NarrativeSource;
use std::{
    io::{Read, Write as _},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
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
fn temp_root(label: &str) -> std::path::PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-runtime-preflight-{label}-{}-{stamp}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create test root");
    root
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

/// A throwaway universe for the operation's unused session parameter, plus
/// the snapshot readers that name the fixed Compaction slots.
fn universe() -> SessionUniverse {
    let root = temp_root("universe");
    let state = root.join("state");
    std::fs::create_dir_all(&state).expect("create the state dir");
    SessionUniverse::compose(&test_runtime(), None, None, false, &root, &state)
        .expect("a test universe composes")
}

fn compaction_cell(snapshot: &LocalPhaseSnapshot) -> PhaseCell {
    let index = phase_index(AgentMetricPhase::Compaction).expect("compaction has a slot");
    snapshot.phases[index]
}

fn started_count(snapshot: &LocalPhaseSnapshot) -> u64 {
    let index = count_index(AgentMetricCount::CompactionStarted).expect("started has a slot");
    snapshot.counts[index]
}

#[tokio::test]
async fn a_compacting_run_records_one_succeeded_sample_and_one_start() {
    let seen = Arc::new(Mutex::new(0_usize));
    let (release_tx, release_rx) = mpsc::channel();
    let (base_url, source) = counting_source(Arc::clone(&seen), release_rx);
    let runtime = runtime_for_source(base_url, Some(64));
    let mut state = openai_turns(4);
    let metrics = LocalMetrics::new();
    let result = run_with_trigger(&runtime, &mut state, &universe(), false, Some(&metrics)).await;
    release_tx.send(()).expect("release source");
    source.join().expect("source thread joins");
    assert!(
        !result.failed,
        "the compaction succeeds: {}",
        result.message
    );
    let snapshot = metrics.snapshot();
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
    assert_eq!(
        *seen.lock().expect("request count lock"),
        1,
        "exactly one provider request was made"
    );
}

#[tokio::test]
async fn a_refusing_provider_records_one_failed_sample() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a port");
    let address = listener.local_addr().expect("the closed port's address");
    drop(listener);
    let runtime = runtime_for_source(format!("http://{address}/v1"), Some(64));
    let mut state = openai_turns(4);
    let metrics = LocalMetrics::new();
    let result = run_with_trigger(&runtime, &mut state, &universe(), false, Some(&metrics)).await;
    assert!(
        result.failed,
        "a dead provider fails the compaction: {}",
        result.message
    );
    let snapshot = metrics.snapshot();
    let cell = compaction_cell(&snapshot);
    assert_eq!(cell.samples, 1, "one compaction sample: {snapshot:?}");
    let failed = outcome_index(AgentMetricOutcome::Failed).expect("failed has a slot");
    assert_eq!(
        cell.outcomes[failed], 1,
        "the sample is Failed: {snapshot:?}"
    );
    assert_eq!(started_count(&snapshot), 1, "the summariser call did start");
}

#[tokio::test]
async fn a_short_conversation_records_one_skipped_sample_and_no_start() {
    let seen = Arc::new(Mutex::new(0_usize));
    let (release_tx, release_rx) = mpsc::channel();
    let (base_url, source) = counting_source(Arc::clone(&seen), release_rx);
    let runtime = runtime_for_source(base_url, Some(64));
    let mut state = openai_turns(2);
    let metrics = LocalMetrics::new();
    let result = run_with_trigger(&runtime, &mut state, &universe(), false, Some(&metrics)).await;
    release_tx.send(()).expect("release source");
    source.join().expect("source thread joins");
    assert!(
        !result.failed && result.summary.is_none(),
        "a short conversation does no work: {}",
        result.message
    );
    let snapshot = metrics.snapshot();
    let cell = compaction_cell(&snapshot);
    assert_eq!(cell.samples, 1, "one compaction sample: {snapshot:?}");
    let skipped = outcome_index(AgentMetricOutcome::Skipped).expect("skipped has a slot");
    assert_eq!(
        cell.outcomes[skipped], 1,
        "the sample is Skipped: {snapshot:?}"
    );
    assert_eq!(
        started_count(&snapshot),
        0,
        "the summariser call never started"
    );
    assert_eq!(
        *seen.lock().expect("request count lock"),
        0,
        "a no-work result never reaches the provider"
    );
}

#[tokio::test]
async fn a_collector_less_run_matches_the_recorded_run() {
    let seen = Arc::new(Mutex::new(0_usize));
    let (release_tx, release_rx) = mpsc::channel();
    let (base_url, source) = counting_source(Arc::clone(&seen), release_rx);
    let runtime = runtime_for_source(base_url, Some(64));
    let mut bare = openai_turns(4);
    let without = run_with_trigger(&runtime, &mut bare, &universe(), false, None).await;
    let mut tracked = openai_turns(4);
    let metrics = LocalMetrics::new();
    let with = run_with_trigger(&runtime, &mut tracked, &universe(), false, Some(&metrics)).await;
    release_tx.send(()).expect("release source");
    source.join().expect("source thread joins");
    assert_eq!(without.message, with.message, "the message is identical");
    assert_eq!(without.failed, with.failed, "the outcome is identical");
    assert_eq!(
        without.compacted_turns, with.compacted_turns,
        "the compacted-turn count is identical"
    );
    assert_eq!(
        *seen.lock().expect("request count lock"),
        2,
        "each run called once"
    );
}

/// End-to-end: a compacting run saves a typed continuation on the state, with
/// the provider's summary as the generated narrative and the persisted turn
/// count as the source boundary.
#[tokio::test]
async fn a_compacting_run_saves_a_continuation_with_the_summary_as_narrative() {
    let seen = Arc::new(Mutex::new(0_usize));
    let (release_tx, release_rx) = mpsc::channel();
    let (base_url, source) = counting_source(Arc::clone(&seen), release_rx);
    let runtime = runtime_for_source(base_url, Some(64));
    let mut state = openai_turns(4);
    let result = run_with_trigger(&runtime, &mut state, &universe(), false, None).await;
    release_tx.send(()).expect("release source");
    source.join().expect("source thread joins");
    assert!(
        !result.failed,
        "the compaction succeeds: {}",
        result.message
    );
    let Some(StoredContinuation::Current(record)) = &state.continuation else {
        panic!(
            "the compaction saved a typed continuation: {:?}",
            state.continuation
        );
    };
    let narrative = record
        .narrative
        .as_ref()
        .expect("the clean summary becomes the narrative");
    assert_eq!(
        narrative.text, "done.",
        "the narrative is the provider's summary"
    );
    assert_eq!(narrative.source, NarrativeSource::Summarizer);
    assert_eq!(
        record.source.through_turn, 4,
        "the boundary is the persisted turn count"
    );
    record.validate().expect("the record validates");
}
