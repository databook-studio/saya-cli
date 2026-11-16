use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentLimits, AgentRequest, AllowReadOnlyApproval,
    CancellationToken, ChatProvider, ChatRequest, ChatResponse, OpenAiCompatibleProvider,
    ProviderError, ProviderRecoveryPhase, ProviderSettings, ProviderStream, ToolError,
    ToolExecutor, run_agent_with_sink,
};
use std::{
    io::{ErrorKind, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

/// The eight-entry default schedule, in order.
pub const DEFAULT_DELAYS: [Duration; 8] = [
    Duration::from_millis(500),
    Duration::from_millis(1000),
    Duration::from_millis(2000),
    Duration::from_millis(4000),
    Duration::from_millis(8000),
    Duration::from_millis(16000),
    Duration::from_millis(30000),
    Duration::from_millis(30000),
];

struct NoTools;

#[async_trait]
impl ToolExecutor for NoTools {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        unreachable!("the provider never answers, so no tool is called")
    }
}

#[derive(Default)]
struct RecordingSink(Mutex<Vec<AgentEvent>>);

#[async_trait]
impl AgentEventSink for RecordingSink {
    async fn emit(&self, event: AgentEvent) {
        self.0.lock().unwrap().push(event);
    }
}

fn read_request(stream: &mut TcpStream) {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let count = stream.read(&mut buffer).unwrap();
        bytes.extend_from_slice(&buffer[..count]);
        let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") else {
            continue;
        };
        let header = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
        let length = header
            .lines()
            .find_map(|line| line.strip_prefix("content-length: "))
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        if bytes.len() >= end + 4 + length {
            return;
        }
    }
}

/// Serves every request with a cut stream that delivered nothing (an empty
/// delta, then the connection closes with no terminator) until told to stop,
/// and returns when each request arrived. A deadline keeps a stuck test from
/// hanging. The stream delivers nothing on purpose: a cut *after* content is a
/// stream cut, which continues instead of retrying, so it would never exercise
/// the schedule this suite is about.
fn cut_stream_server(stop: Arc<AtomicBool>) -> (String, thread::JoinHandle<Vec<Instant>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut arrivals = Vec::new();
        while !stop.load(Ordering::SeqCst) && Instant::now() < deadline {
            let mut stream = match listener.accept() {
                Ok((stream, _)) => stream,
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(1));
                    continue;
                }
                Err(error) => panic!("accept failed: {error}"),
            };
            arrivals.push(Instant::now());
            stream.set_nonblocking(false).unwrap();
            read_request(&mut stream);
            let body = "data: {\"choices\":[{\"delta\":{}}]}\n\n";
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
        arrivals
    });
    (base, handle)
}

/// What one agent turn against always-cut streams did.
pub struct Run {
    /// When each request reached the server.
    pub arrivals: Vec<Instant>,
    /// Each provider-recovery event as (phase, attempt, limit).
    pub recoveries: Vec<(ProviderRecoveryPhase, u8, u8)>,
}

impl Run {
    /// The time between consecutive attempts.
    pub fn gaps(&self) -> Vec<Duration> {
        self.arrivals
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .collect()
    }
}

/// Runs one turn that must end in a provider failure and returns the
/// provider-recovery events it reported.
async fn failing_turn(provider: &dyn ChatProvider) -> Vec<(ProviderRecoveryPhase, u8, u8)> {
    let sink = RecordingSink::default();
    let outcome = run_agent_with_sink(
        provider,
        &NoTools,
        AgentRequest {
            prompt: "show data".into(),
            profile_names: Vec::new(),
            model: "test-model".into(),
            system_prompt: None,
            history: Vec::new(),
            context_blocks: Vec::new(),
        },
        Vec::new(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &sink,
        CancellationToken::new(),
    )
    .await;
    assert!(
        outcome.is_err(),
        "a stream that never completes is a failure"
    );
    sink.0
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            AgentEvent::ProviderRecovery {
                phase,
                attempt,
                limit,
                ..
            } => Some((*phase, *attempt, *limit)),
            _ => None,
        })
        .collect()
}

/// The schedule a provider built from unconfigured settings reports, without
/// running a turn: the default sleeps about ninety seconds in total, far too
/// long for a test that waits it out.
pub fn unset_schedule() -> Vec<Duration> {
    let settings = ProviderSettings::new("test-model", Some("http://127.0.0.1:1".into()));
    OpenAiCompatibleProvider::new(settings, Some("test-key"))
        .unwrap()
        .retry_delays()
}

/// Runs one turn against a real provider whose streams are always cut. `None`
/// leaves its retry schedule unconfigured.
pub async fn run_against_cut_streams(delays: Option<Vec<Duration>>) -> Run {
    let stop = Arc::new(AtomicBool::new(false));
    let (base, server) = cut_stream_server(stop.clone());
    let mut settings = ProviderSettings::new("test-model", Some(base));
    if let Some(delays) = delays {
        settings = settings.with_retry_delays(delays);
    }
    let provider = OpenAiCompatibleProvider::new(settings, Some("test-key")).unwrap();
    let recoveries = failing_turn(&provider).await;
    stop.store(true, Ordering::SeqCst);
    Run {
        arrivals: server.join().unwrap(),
        recoveries,
    }
}

/// A provider that is not built from validated settings, so it can declare a
/// schedule longer than any real provider accepts. Its streams are always cut
/// without delivering anything.
struct ScheduledProvider {
    delays: Vec<Duration>,
    arrivals: Mutex<Vec<Instant>>,
}

#[async_trait]
impl ChatProvider for ScheduledProvider {
    fn name(&self) -> &str {
        "scheduled"
    }
    fn retry_delays(&self) -> Vec<Duration> {
        self.delays.clone()
    }
    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the loop drives the provider through stream")
    }
    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        self.arrivals.lock().unwrap().push(Instant::now());
        // Delivers nothing, so the cut stays retryable and the schedule runs.
        Ok(Box::pin(futures_util::stream::iter(vec![Err(
            ProviderError::StreamEnded,
        )])))
    }
}

/// Runs one turn against a provider that declares `delays` itself and whose
/// streams are always cut.
pub async fn run_with_declared_schedule(delays: Vec<Duration>) -> Run {
    let provider = ScheduledProvider {
        delays,
        arrivals: Mutex::new(Vec::new()),
    };
    let recoveries = failing_turn(&provider).await;
    let arrivals = provider.arrivals.lock().unwrap().clone();
    Run {
        arrivals,
        recoveries,
    }
}
