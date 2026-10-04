//! Production preflight at the real runtime boundary.
//!
//! A too-small known `context_window_tokens` refuses before any provider
//! request; a generous one dispatches. Uses the real production entry
//! (`run_prompt_with_sink_and_outcomes`, which builds the live HTTP provider
//! from config) against a local counting source and a real SQLite connector,
//! so the real tool definitions/schemas are in the request.

use crate::agent::runtime::{
    PromptOverrides, QuestionScope, SinkPrompt, run_prompt_with_sink_and_outcomes,
};
use crate::config::runtime::RuntimeConfig;
use async_trait::async_trait;
use saya_agent::{AgentEvent, AgentEventSink, AgentMode, ApprovalPolicy, BudgetDimension};
use saya_config::{AiProvider, ConnectionsFile, ResolvedAi};
use saya_types::DatabaseProfile;
use std::{
    collections::BTreeMap,
    io::{Read, Write as _},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[path = "runtime_preflight_second_tests.rs"]
mod second_tests;

/// A local HTTP source counting every request it receives.
pub(super) fn counting_source(
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

pub(super) fn temp_root(label: &str) -> std::path::PathBuf {
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

pub(super) struct RecordingSink {
    pub(super) events: Arc<Mutex<Vec<AgentEvent>>>,
}

#[async_trait]
impl AgentEventSink for RecordingSink {
    async fn emit(&self, event: AgentEvent) {
        self.events.lock().expect("event log lock").push(event);
    }
}

pub(super) async fn runtime_for_source(
    root: &std::path::Path,
    base_url: String,
    context_window_tokens: Option<u64>,
) -> RuntimeConfig {
    let database = root.join("preflight.sqlite3");
    let seed = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&database)
            .create_if_missing(true),
    )
    .await
    .expect("open synthetic SQLite database");
    sqlx::query("CREATE TABLE orders (id INTEGER)")
        .execute(&seed)
        .await
        .expect("seed synthetic SQLite table");
    sqlx::query("INSERT INTO orders (id) VALUES (1)")
        .execute(&seed)
        .await
        .expect("seed synthetic SQLite row");
    seed.close().await;
    let profile = DatabaseProfile::Sqlite {
        path: database.display().to_string(),
        read_only: true,
    };
    let mut runtime = super::test_runtime(super::default_memory());
    runtime.resolved.profile_name = Some("analytics".into());
    runtime.resolved.profile = Some(profile.clone());
    runtime.resolved.ai = ResolvedAi {
        provider: AiProvider::OpenaiCompatible,
        model: "preflight-test".into(),
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
    runtime.connections = ConnectionsFile {
        profiles: BTreeMap::from([("analytics".into(), profile)]),
    };
    runtime.connections_path = Some(root.join("connections.toml"));
    runtime.cache_scope = root.to_path_buf();
    runtime.investigations_root = root.join("investigations");
    runtime
}

pub(super) const ANSWER_RESPONSE: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"done.\"},\"finish_reason\":\"stop\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":1}}\n\ndata: [DONE]\n\n";

#[tokio::test]
async fn a_too_small_known_window_refuses_before_any_provider_request() {
    let root = temp_root("refuse");
    let seen = Arc::new(Mutex::new(0_usize));
    let (release_tx, release_rx) = mpsc::channel();
    let (base_url, source) = counting_source(Arc::clone(&seen), release_rx);
    let runtime = runtime_for_source(&root, base_url, Some(64)).await;
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: Arc::clone(&events),
    };
    let cancellation = saya_agent::CancellationToken::new();
    let output = run_prompt_with_sink_and_outcomes(SinkPrompt {
        runtime: &runtime,
        prompt: "count rows",
        approval: ApprovalPolicy::ReadOnly,
        can_prompt: false,
        can_obtain_approval: false,
        overrides: PromptOverrides::default(),
        history: Vec::new(),
        sink: &sink,
        cancellation: cancellation.clone(),
        question_scope: None,
        state_db: None,
        decider: None,
        last_sql: None,
        session: None,
        agent_mode: AgentMode::Build,
        capture: None,
        prior_tool_outcomes: None,
        compaction_summary: None,
    })
    .await
    .expect("a preflight refusal is truncated output, not an error");
    release_tx.send(()).expect("release source");
    source.join().expect("source thread joins");
    assert_eq!(
        *seen.lock().expect("request count lock"),
        0,
        "a too-small known window refuses before any provider request"
    );
    assert!(
        output.truncated,
        "the refusal is truncated output: {output:?}"
    );
    let stops: Vec<BudgetDimension> = events
        .lock()
        .expect("event log lock")
        .iter()
        .filter_map(|event| match event {
            AgentEvent::AnsweringBudgetStopped { dimension } => Some(*dimension),
            _ => None,
        })
        .collect();
    assert_eq!(
        stops,
        vec![BudgetDimension::RequestContextWindow],
        "exactly one RequestContextWindow stop: {stops:?}"
    );
    assert!(
        !output.used_bounded_sql_query,
        "no tool/SQL effect happened: {output:?}"
    );
    assert_eq!(
        cancellation.reason(),
        Some(saya_agent::CancellationReason::Budget(
            BudgetDimension::RequestContextWindow
        )),
        "the refusal latches the typed budget reason"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn a_generous_window_dispatches_normally() {
    second_tests::generous_dispatch().await;
}

#[tokio::test]
async fn the_shared_question_scope_survives_a_preflight_refusal() {
    second_tests::scope_survives_refusal().await;
}
