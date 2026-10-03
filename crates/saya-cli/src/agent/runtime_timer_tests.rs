//! Production-boundary coverage for the answering clock's extraction scope.

use super::{RecordingSink, assisted_memory, test_runtime};
use crate::{
    agent::runtime::{PromptOverrides, SinkPrompt, run_prompt_with_sink_and_outcomes},
    config::runtime::RuntimeConfig,
};
use saya_agent::{
    AgentEvent, AgentMode, ApprovalPolicy, BudgetDimension, CancellationToken, TokenUsage,
};
use saya_config::{
    AiProvider, BudgetLimit, ConnectionsFile, ResolvedAi, ResolvedInvestigationBudgets,
};
use saya_store::SqliteStateStore;
use saya_types::DatabaseProfile;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    num::NonZeroU64,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestKind {
    Tool,
    Answer,
    Extraction,
}

/// The local source is deliberately sequential: the third observed request is
/// accepted only when it carries the extraction system marker. Its release is
/// the natural bounded completion of the ancillary stream, not a test seam in
/// the runtime.
fn timer_source(
    extraction_started: tokio::sync::oneshot::Sender<()>,
    release: mpsc::Receiver<()>,
    seen: Arc<Mutex<Vec<RequestKind>>>,
) -> (String, thread::JoinHandle<Result<(), String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind timer source");
    let address = listener.local_addr().expect("timer source address");
    let handle = thread::spawn(move || {
        let mut extraction_started = Some(extraction_started);
        for expected in [
            RequestKind::Tool,
            RequestKind::Answer,
            RequestKind::Extraction,
        ] {
            let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
            let body = read_request(&mut stream)?;
            let kind = classify_request(&body)?;
            if kind != expected {
                return Err(format!("expected {expected:?}, received {kind:?}"));
            }
            seen.lock()
                .map_err(|_| "request log poisoned".to_string())?
                .push(kind);
            match kind {
                RequestKind::Tool => respond(&mut stream, TOOL_RESPONSE)?,
                RequestKind::Answer => respond(&mut stream, ANSWER_RESPONSE)?,
                RequestKind::Extraction => {
                    extraction_started
                        .take()
                        .expect("only one extraction source request")
                        .send(())
                        .map_err(|_| "runtime stopped before extraction observation".to_string())?;
                    release
                        .recv_timeout(Duration::from_secs(4))
                        .map_err(|_| "test did not release extraction".to_string())?;
                    respond(&mut stream, EXTRACTION_RESPONSE)?;
                }
            }
        }
        Ok(())
    });
    (format!("http://{address}/v1"), handle)
}

fn read_request(stream: &mut TcpStream) -> Result<String, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let (header_end, content_length) = loop {
        let read = stream
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("source request ended before headers".into());
        }
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let header = std::str::from_utf8(&bytes[..end]).map_err(|error| error.to_string())?;
            let length = header
                .lines()
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .map(|(_, value)| value.trim())
                })
                .ok_or_else(|| "source request omitted content length".to_string())?
                .parse::<usize>()
                .map_err(|error| error.to_string())?;
            break (end + 4, length);
        }
    };
    while bytes.len() < header_end + content_length {
        let read = stream
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("source request ended before body".into());
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    String::from_utf8(bytes[header_end..header_end + content_length].to_vec())
        .map_err(|error| error.to_string())
}

fn classify_request(body: &str) -> Result<RequestKind, String> {
    let request: serde_json::Value =
        serde_json::from_str(body).map_err(|error| error.to_string())?;
    let messages = request["messages"]
        .as_array()
        .ok_or_else(|| "source request omitted messages".to_string())?;
    let system = messages
        .first()
        .and_then(|message| message["content"].as_str())
        .ok_or_else(|| "source request omitted system text".to_string())?;
    if system.contains("precision schema knowledge extractor") {
        return Ok(RequestKind::Extraction);
    }
    let tools = request["tools"]
        .as_array()
        .ok_or_else(|| "answering request omitted tools".to_string())?;
    let has_sql = tools.iter().any(|tool| {
        tool["function"]["name"]
            .as_str()
            .is_some_and(|name| name == "bounded_sql_query")
    });
    if !has_sql {
        return Err("answering request did not advertise bounded_sql_query".into());
    }
    if messages.iter().any(|message| message["role"] == "tool") {
        Ok(RequestKind::Answer)
    } else {
        Ok(RequestKind::Tool)
    }
}

fn respond(stream: &mut TcpStream, body: &str) -> Result<(), String> {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|error| error.to_string())
}

async fn runtime_for_timer_source(root: &std::path::Path, base_url: String) -> RuntimeConfig {
    let database = root.join("timer.sqlite3");
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
    let mut runtime = test_runtime(assisted_memory());
    runtime.resolved.profile_name = Some("analytics".into());
    runtime.resolved.profile = Some(profile.clone());
    runtime.resolved.ai = ResolvedAi {
        provider: AiProvider::OpenaiCompatible,
        model: "timer-test".into(),
        base_url: Some(base_url),
        api_key: None,
        allow_data_sharing: true,
        temperature: 0.0,
        timeout_seconds: 2,
        idle_timeout_seconds: 4,
        max_output_tokens: 128,
        max_output_tokens_is_default: false,
        context_byte_budget: 256 * 1024,
        context_window_tokens: None,
        show_thinking: false,
        compaction: saya_config::CompactionMode::Auto,
        retry_delays_ms: Vec::new(),
    };
    runtime.resolved.investigation_budgets = ResolvedInvestigationBudgets {
        logical_answering_requests: finite(8),
        requested_tool_calls: finite(8),
        elapsed_seconds: finite(1),
        known_reported_tokens: finite(100),
    };
    runtime.connections = ConnectionsFile {
        profiles: BTreeMap::from([("analytics".into(), profile)]),
    };
    runtime.connections_path = Some(root.join("connections.toml"));
    runtime.cache_scope = root.to_path_buf();
    runtime.investigations_root = root.join("investigations");
    runtime
}

const fn finite(value: u64) -> BudgetLimit {
    BudgetLimit::Finite(NonZeroU64::new(value).expect("test budget is nonzero"))
}

fn temp_root() -> std::path::PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-runtime-answering-timer-{}-{stamp}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create test root");
    root
}

#[tokio::test]
async fn answering_deadline_disarms_before_source_observed_extraction() {
    let root = temp_root();
    let (extraction_started_tx, extraction_started) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (base_url, source) = timer_source(extraction_started_tx, release_rx, Arc::clone(&seen));
    let runtime = runtime_for_timer_source(&root, base_url).await;
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: Arc::clone(&events),
        knowledge_log: Arc::new(Mutex::new(Vec::new())),
    };
    let cancellation = CancellationToken::new();
    let state = SqliteStateStore::new(root.join("state.sqlite3"));
    let work = run_prompt_with_sink_and_outcomes(SinkPrompt {
        runtime: &runtime,
        prompt: "Remember that analytics is the default source, then find a value.",
        approval: ApprovalPolicy::ReadOnly,
        can_prompt: false,
        can_obtain_approval: false,
        overrides: PromptOverrides::default(),
        history: Vec::new(),
        sink: &sink,
        cancellation: cancellation.clone(),
        state_db: Some(state),
        decider: None,
        last_sql: None,
        session: None,
        agent_mode: AgentMode::Build,
        capture: None,
        prior_tool_outcomes: None,
        compaction_summary: None,
    });
    tokio::pin!(work);

    let extraction_wait = tokio::time::timeout(Duration::from_secs(3), extraction_started);
    tokio::pin!(extraction_wait);
    tokio::select! {
        source_wait = &mut extraction_wait => match source_wait {
            Ok(Ok(())) => {}
            Ok(Err(_)) => panic!(
                "timer source stopped before extraction observation: {:?}",
                source.join().expect("timer source thread joins")
            ),
            Err(_) => panic!("real extraction request did not reach the local source"),
        },
        result = &mut work => panic!(
            "runtime settled before source-observed extraction: {result:?}; source: {:?}; events: {:?}",
            source.join().expect("timer source thread joins"),
            events.lock().expect("event log lock")
        ),
    }

    // The source, not a sleep, establishes extraction is now pending. Holding
    // it longer than the one-second answering budget catches a timer that
    // remains armed for ancillary work.
    let past_answering_deadline = tokio::time::sleep(Duration::from_millis(1200));
    tokio::pin!(past_answering_deadline);
    tokio::select! {
        _ = &mut past_answering_deadline => {}
        result = &mut work => panic!(
            "runtime settled while the extraction source was intentionally pending: {result:?}"
        ),
    }
    assert_eq!(
        cancellation.reason(),
        None,
        "the answering clock must be disarmed while extraction is pending"
    );

    release_tx
        .send(())
        .expect("release bounded extraction source");
    let output = tokio::time::timeout(Duration::from_secs(3), &mut work)
        .await
        .expect("runtime settles after extraction source release")
        .expect("answering and extraction complete");
    source
        .join()
        .expect("timer source thread joins")
        .expect("timer source handled the expected requests");

    assert_eq!(output.answer, "The query returned one.");
    assert_eq!(output.usage, TokenUsage::new(7, 2));
    assert_eq!(output.learning_usage, Some(TokenUsage::new(3, 1)));
    assert_eq!(
        *seen.lock().expect("request log lock"),
        vec![
            RequestKind::Tool,
            RequestKind::Answer,
            RequestKind::Extraction
        ],
        "one tool-bearing answering round, one final answering round, then one extraction"
    );
    let events = events.lock().expect("event log lock");
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::ToolCompleted { name, .. } if name == "bounded_sql_query"
    )));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AgentEvent::KnowledgeLearningStarted))
    );
    assert!(
        !events.iter().any(|event| matches!(
            event,
            AgentEvent::AnsweringBudgetStopped {
                dimension: BudgetDimension::ElapsedTime
            }
        )),
        "ancillary extraction does not add an answering budget stop"
    );
    let _ = std::fs::remove_dir_all(root);
}

const TOOL_RESPONSE: &str = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-sql\",\"function\":{\"name\":\"bounded_sql_query\",\"arguments\":\"{\\\"connection\\\":\\\"analytics\\\",\\\"sql\\\":\\\"SELECT id FROM orders\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":1}}\n\ndata: [DONE]\n\n";
const ANSWER_RESPONSE: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"The query returned one.\"},\"finish_reason\":\"stop\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":1}}\n\ndata: [DONE]\n\n";
const EXTRACTION_RESPONSE: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"{\\\"proposals\\\": []}\"},\"finish_reason\":\"stop\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":1}}\n\ndata: [DONE]\n\n";
