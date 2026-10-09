//! Producer regression for candidate receipts: actual provider events, an
//! actual SQLite tool call, and a protocol-rejected first candidate.

use super::super::question_scope::QuestionScope;
use super::super::runtime::PromptOverrides;
use super::run_with_candidates;
use crate::config::runtime::RuntimeConfig;
use async_trait::async_trait;
use saya_agent::{AgentEvent, AgentEventSink, AgentMode, ApprovalPolicy, CancellationToken};
use saya_config::{
    AiProvider, BudgetLimit, ColorChoice, ConnectionsFile, OutputFormat, ResolvedAi,
    ResolvedConfig, ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs,
    ResolvedInvestigationBudgets, ResolvedJobs, ResolvedMemory, ResolvedRunnerJobs, ThemeChoice,
};
use saya_types::DatabaseProfile;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    num::NonZeroU64,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

struct RecordingSink(Arc<Mutex<Vec<AgentEvent>>>);

#[async_trait]
impl AgentEventSink for RecordingSink {
    async fn emit(&self, event: AgentEvent) {
        self.0.lock().expect("event log lock").push(event);
    }
}

fn candidate_source(
    requests: Arc<Mutex<Vec<String>>>,
) -> (String, thread::JoinHandle<Result<(), String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind candidate source");
    listener
        .set_nonblocking(true)
        .expect("candidate source listener becomes nonblocking");
    let address = listener.local_addr().expect("candidate source address");
    let source = thread::spawn(move || {
        for response in [BAD_TOOL_RESPONSE, TOOL_RESPONSE, ANSWER_RESPONSE] {
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(accepted) => break accepted,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if std::time::Instant::now() >= deadline {
                            return Err("candidate source timed out waiting for request".into());
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => return Err(error.to_string()),
                }
            };
            // Accepted sockets inherit O_NONBLOCK from a non-blocking listener on macOS.
            stream
                .set_nonblocking(false)
                .map_err(|error| error.to_string())?;
            let request = read_request(&mut stream)?;
            requests
                .lock()
                .map_err(|_| "request log poisoned".to_string())?
                .push(request);
            respond(&mut stream, response)?;
        }
        Ok(())
    });
    (format!("http://{address}/v1"), source)
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
            let content_length = header
                .lines()
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .map(|(_, value)| value.trim())
                })
                .ok_or_else(|| "source request omitted content length".to_string())?
                .parse::<usize>()
                .map_err(|error| error.to_string())?;
            break (end + 4, content_length);
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

fn respond(stream: &mut TcpStream, body: &str) -> Result<(), String> {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|error| error.to_string())
}

async fn runtime_with_sqlite(root: &Path, base_url: String) -> RuntimeConfig {
    let database = root.join("candidate.sqlite3");
    let pool = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&database)
            .create_if_missing(true),
    )
    .await
    .expect("open synthetic SQLite database");
    sqlx::query("CREATE TABLE orders (id INTEGER)")
        .execute(&pool)
        .await
        .expect("seed synthetic SQLite table");
    sqlx::query("INSERT INTO orders (id) VALUES (1)")
        .execute(&pool)
        .await
        .expect("seed synthetic SQLite row");
    pool.close().await;

    let profile = DatabaseProfile::Sqlite {
        path: database.display().to_string(),
        read_only: true,
    };
    RuntimeConfig {
        resolved: ResolvedConfig {
            instructions: Default::default(),
            profile_name: Some("analytics".into()),
            profile: Some(profile.clone()),
            investigation_budgets: ResolvedInvestigationBudgets {
                logical_answering_requests: finite(8),
                requested_tool_calls: finite(8),
                elapsed_seconds: finite(10),
                known_reported_tokens: finite(1000),
            },
            ai: ResolvedAi {
                provider: AiProvider::OpenaiCompatible,
                model: "candidate-receipt-test".into(),
                base_url: Some(base_url),
                api_key: None,
                allow_data_sharing: true,
                temperature: 0.0,
                timeout_seconds: 2,
                idle_timeout_seconds: 2,
                max_output_tokens: 128,
                max_output_tokens_is_default: false,
                context_byte_budget: 256 * 1024,
                context_window_tokens: None,
                show_thinking: false,
                compaction: saya_config::CompactionMode::Auto,
                retry_delays_ms: Vec::new(),
            },
            max_rows: 100,
            read_only: true,
            max_iterations: 4,
            candidates: 2,
            jobs: ResolvedJobs {
                wall_clock_seconds: None,
                tokens_per_endpoint: BTreeMap::new(),
                turns: Some(4),
                tool_calls: None,
                fetch: ResolvedFetchJobs::default(),
                interpreter: ResolvedInterpreterJobs::default(),
                runner: ResolvedRunnerJobs::default(),
            },
            query_timeout_seconds: 2,
            output_format: OutputFormat::Text,
            output_color: ColorChoice::Never,
            ui_theme: ThemeChoice::Auto,
            memory: ResolvedMemory {
                mode: saya_config::MemoryMode::Off,
                max_contracts: 5,
                max_claims_per_contract: 12,
                max_context_bytes: 16_384,
            },
            host_commands: ResolvedHostCommands::default(),
            session_deny: Default::default(),
            ignored_project_overrides: Vec::new(),
            provenance: Default::default(),
            endpoints: BTreeMap::new(),
        },
        connections: ConnectionsFile {
            profiles: BTreeMap::from([("analytics".into(), profile)]),
        },
        config_path: None,
        connections_path: Some(root.join("connections.toml")),
        cache_scope: root.to_path_buf(),
        investigations_root: root.join("investigations"),
        secret_values: BTreeMap::new(),
    }
}

const fn finite(value: u64) -> BudgetLimit {
    BudgetLimit::Finite(NonZeroU64::new(value).expect("test budget is nonzero"))
}

fn temp_root() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-candidate-receipt-{}-{stamp}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create test root");
    root
}

const BAD_TOOL_RESPONSE: &str = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"duplicate\",\"function\":{\"name\":\"bounded_sql_query\",\"arguments\":\"{\\\"connection\\\":\\\"analytics\\\",\\\"sql\\\":\\\"SELECT id FROM orders\\\"}\"}},{\"index\":1,\"id\":\"duplicate\",\"function\":{\"name\":\"bounded_sql_query\",\"arguments\":\"{\\\"connection\\\":\\\"analytics\\\",\\\"sql\\\":\\\"SELECT id FROM orders\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":13}}\n\ndata: [DONE]\n\n";
const TOOL_RESPONSE: &str = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-sql\",\"function\":{\"name\":\"bounded_sql_query\",\"arguments\":\"{\\\"connection\\\":\\\"analytics\\\",\\\"sql\\\":\\\"SELECT id FROM orders\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":17,\"completion_tokens\":19}}\n\ndata: [DONE]\n\n";
const ANSWER_RESPONSE: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"The query returned one.\"},\"finish_reason\":\"stop\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":23,\"completion_tokens\":29}}\n\ndata: [DONE]\n\n";

#[path = "receipt_tests/candidate_receipt.rs"]
mod candidate_receipt;
