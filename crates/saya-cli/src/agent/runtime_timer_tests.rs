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
    listener
        .set_nonblocking(true)
        .expect("timer source listener becomes nonblocking");
    let address = listener.local_addr().expect("timer source address");
    let handle = thread::spawn(move || {
        let mut extraction_started = Some(extraction_started);
        for expected in [
            RequestKind::Tool,
            RequestKind::Answer,
            RequestKind::Extraction,
        ] {
            let accept_deadline = std::time::Instant::now() + Duration::from_secs(4);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(accepted) => break accepted,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if release.try_recv().is_ok() {
                            return Err("timer source released before request".into());
                        }
                        if std::time::Instant::now() >= accept_deadline {
                            return Err("timer source timed out waiting for request".into());
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => return Err(error.to_string()),
                }
            };
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

fn postgres_setup_source(
    setup_started: tokio::sync::oneshot::Sender<()>,
    release: mpsc::Receiver<()>,
) -> (u16, thread::JoinHandle<Result<(), String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind PostgreSQL setup source");
    let port = listener
        .local_addr()
        .expect("PostgreSQL source address")
        .port();
    let source = thread::spawn(move || {
        listener
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
        let deadline = std::time::Instant::now() + Duration::from_secs(4);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if release.try_recv().is_ok() {
                        return Err("setup source released before startup".into());
                    }
                    if std::time::Instant::now() >= deadline {
                        return Err("setup source timed out waiting for startup".into());
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(error.to_string()),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|error| error.to_string())?;
        let mut startup = [0_u8; 32];
        let bytes = stream
            .read(&mut startup)
            .map_err(|error| error.to_string())?;
        if bytes == 0 {
            return Err("PostgreSQL client closed before startup bytes".into());
        }
        setup_started
            .send(())
            .map_err(|_| "runtime stopped before setup observation".to_string())?;
        release
            .recv_timeout(Duration::from_secs(4))
            .map_err(|_| "test did not release PostgreSQL setup".to_string())?;
        Ok(())
    });
    (port, source)
}

async fn runtime_for_postgres_setup(root: &std::path::Path, port: u16) -> RuntimeConfig {
    let profile = DatabaseProfile::Postgres {
        host: "127.0.0.1".into(),
        port: Some(port),
        database: "synthetic".into(),
        user: "saya".into(),
        ssl_mode: Some(saya_types::PostgresSslMode::Disable),
        password: None,
    };
    let mut runtime = test_runtime(assisted_memory());
    runtime.resolved.profile_name = Some("setup".into());
    runtime.resolved.profile = Some(profile.clone());
    runtime.resolved.investigation_budgets = ResolvedInvestigationBudgets {
        logical_answering_requests: finite(8),
        requested_tool_calls: finite(8),
        elapsed_seconds: finite(1),
        known_reported_tokens: finite(100),
    };
    runtime.resolved.query_timeout_seconds = 4;
    runtime.connections = ConnectionsFile {
        profiles: BTreeMap::from([("setup".into(), profile)]),
    };
    runtime.connections_path = Some(root.join("connections.toml"));
    runtime.cache_scope = root.to_path_buf();
    runtime.investigations_root = root.join("investigations");
    runtime
}

#[tokio::test]
async fn answering_deadline_starts_before_postgres_setup_and_awaits_the_source() {
    let root = temp_root();
    let (setup_started_tx, setup_started) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (port, source) = postgres_setup_source(setup_started_tx, release_rx);
    let runtime = runtime_for_postgres_setup(&root, port).await;
    let cancellation = CancellationToken::new();
    let sink = RecordingSink {
        events: Arc::new(Mutex::new(Vec::new())),
        knowledge_log: Arc::new(Mutex::new(Vec::new())),
    };
    let work = run_prompt_with_sink_and_outcomes(SinkPrompt {
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
    });
    tokio::pin!(work);
    let setup_wait = tokio::time::timeout(Duration::from_secs(3), setup_started);
    tokio::pin!(setup_wait);
    tokio::select! {
        setup = &mut setup_wait => match setup {
            Ok(Ok(())) => {}
            Ok(Err(_)) => panic!("runtime stopped before PostgreSQL setup observation"),
            Err(_) => {
                let _ = release_tx.send(());
                panic!(
                    "the production PostgreSQL connector did not send startup bytes: {:?}",
                    source.join().expect("PostgreSQL setup source thread joins")
                );
            }
        },
        result = &mut work => panic!("runtime settled before PostgreSQL setup observation: {result:?}"),
    }

    tokio::select! {
        _ = tokio::time::sleep(Duration::from_millis(1200)) => {}
        result = &mut work => panic!("runtime settled before setup source release: {result:?}"),
    }
    assert_eq!(
        cancellation.reason(),
        Some(saya_agent::CancellationReason::Budget(
            BudgetDimension::ElapsedTime
        )),
        "the supplied token latches while prepare_turn is still in PostgreSQL setup"
    );
    assert!(
        sink.events.lock().expect("event log lock").is_empty(),
        "setup cancellation reaches no provider or tool event"
    );

    release_tx
        .send(())
        .expect("release PostgreSQL setup source");
    let result = tokio::time::timeout(Duration::from_secs(3), &mut work)
        .await
        .expect("runtime awaits source settlement after cancellation");
    assert!(
        result.is_err(),
        "the released synthetic setup must fail the exact runtime"
    );
    source
        .join()
        .expect("PostgreSQL setup source thread joins")
        .expect("PostgreSQL setup source observed and released one startup");
    let _ = std::fs::remove_dir_all(root);
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
        question_scope: None,
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
            Ok(Err(_)) => panic!("timer source stopped before extraction observation"),
            Err(_) => panic!("real extraction request did not reach the local source"),
        },
        result = &mut work => panic!(
            "runtime settled before source-observed extraction: {result:?}; events: {:?}",
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

#[tokio::test]
async fn pre_stopped_question_never_opens_candidate_setup_connection() {
    for candidates in [1, 2] {
        let root = temp_root();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut runtime = runtime_for_postgres_setup(&root, port).await;
        runtime.resolved.query_timeout_seconds = 1;
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let scope =
            crate::agent::question_scope::QuestionScope::new(&runtime, cancellation.clone());
        let sink = RecordingSink {
            events: Arc::new(Mutex::new(Vec::new())),
            knowledge_log: Arc::new(Mutex::new(Vec::new())),
        };
        let result = tokio::time::timeout(
            Duration::from_secs(4),
            crate::agent::candidates::run_with_candidates(
                &runtime,
                "do not start a stopped question",
                ApprovalPolicy::ReadOnly,
                false,
                false,
                PromptOverrides::default(),
                Vec::new(),
                &sink,
                scope,
                None,
                None,
                None,
                None,
                AgentMode::Build,
                candidates,
            ),
        )
        .await
        .expect("stopped candidate entry settles without source work");
        assert!(result.is_err());
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
            "a stopped question must not open a new setup connection ({candidates} candidates)"
        );
        assert!(sink.events.lock().unwrap().is_empty());
        assert_eq!(
            cancellation.reason(),
            Some(saya_agent::CancellationReason::User)
        );
        let _ = std::fs::remove_dir_all(root);
    }
}

#[tokio::test]
async fn supplied_unlimited_question_does_not_inherit_a_new_setup_deadline() {
    let root = temp_root();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (port, source) = postgres_setup_source(started_tx, release_rx);
    let mut runtime = runtime_for_postgres_setup(&root, port).await;
    runtime.resolved.investigation_budgets.elapsed_seconds = BudgetLimit::Unlimited;
    let cancellation = CancellationToken::new();
    let scope = crate::agent::question_scope::QuestionScope::new(&runtime, cancellation.clone());
    runtime.resolved.investigation_budgets.elapsed_seconds = finite(1);
    let sink = RecordingSink {
        events: Arc::new(Mutex::new(Vec::new())),
        knowledge_log: Arc::new(Mutex::new(Vec::new())),
    };
    let work = run_prompt_with_sink_and_outcomes(SinkPrompt {
        runtime: &runtime,
        prompt: "a frozen unlimited question",
        approval: ApprovalPolicy::ReadOnly,
        can_prompt: false,
        can_obtain_approval: false,
        overrides: PromptOverrides::default(),
        history: Vec::new(),
        sink: &sink,
        cancellation: cancellation.clone(),
        question_scope: Some(scope),
        state_db: None,
        decider: None,
        last_sql: None,
        session: None,
        agent_mode: AgentMode::Build,
        capture: None,
        prior_tool_outcomes: None,
        compaction_summary: None,
    });
    tokio::pin!(work);
    let started = tokio::time::timeout(Duration::from_secs(3), started_rx);
    tokio::pin!(started);
    tokio::select! {
        observed = &mut started => observed.unwrap().unwrap(),
        result = &mut work => panic!("work settled before the setup source: {result:?}"),
    }
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_millis(1200)) => {},
        result = &mut work => panic!("work settled before source release: {result:?}"),
    }
    let reason_before_release = cancellation.reason();
    release_tx.send(()).unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(3), &mut work)
            .await
            .unwrap()
            .is_err()
    );
    source.join().unwrap().unwrap();
    assert_eq!(
        reason_before_release, None,
        "the supplied unlimited scope owns the clock despite a different fresh configuration"
    );
    assert_eq!(cancellation.reason(), None);
    let _ = std::fs::remove_dir_all(root);
}
