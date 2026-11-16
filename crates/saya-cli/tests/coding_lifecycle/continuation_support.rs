use super::super::support::Fixture;
use saya_store::NewCodingChild;
use saya_types::{
    AgentId, CodingEventKind, CodingEventV1, CodingResultOutcome, InvocationId, JobId, JobState,
    JoinedResultV1,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    num::NonZeroU64,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// Appends a bounded batch of very large pending assignments to the job's task
/// history: the projection must omit their bytes rather than carry them.
pub(super) fn append_large_pending_tasks(fixture: &Fixture, job: &JobId) {
    let tasks = (0..5)
        .map(|index| {
            json!({
                "agent_id": format!("pending-{index:02}"),
                "task": "x".repeat(16 * 1024),
                "write_paths": [],
                "dependencies": []
            })
        })
        .collect::<Vec<_>>();
    let path = fixture
        .root
        .join("coding")
        .join(job.as_str())
        .join("tasks.jsonl");
    let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    writeln!(file, "{}", serde_json::to_string(&tasks).unwrap()).unwrap();
}

/// Records 34 durable completions the operator never accepted, so the resumed
/// root's continuation projection has a bounded replay to carry.
pub(super) async fn append_completion_history(fixture: &Fixture, job: &JobId) -> NonZeroU64 {
    let events = fixture
        .store
        .list_coding_events(job, None, 100)
        .await
        .unwrap();
    let mut sequence = events.events.last().unwrap().sequence().get();
    for index in 0..34 {
        sequence += 1;
        let agent = AgentId::parse(&format!("history-{index:02}")).unwrap();
        let invocation = InvocationId::parse(&format!("inv-history-{index:02}")).unwrap();
        fixture
            .store
            .create_coding_child(NewCodingChild {
                job_id: job.clone(),
                agent_id: agent.clone(),
                current_invocation_id: invocation.clone(),
            })
            .await
            .unwrap();
        let running = CodingEventV1::new(
            NonZeroU64::new(sequence).unwrap(),
            job.clone(),
            Some(agent.clone()),
            Some(invocation.clone()),
            CodingEventKind::StateChanged {
                state: JobState::Running,
            },
        )
        .unwrap();
        fixture.store.append_coding_event(running).await.unwrap();
        sequence += 1;
        let sequence = NonZeroU64::new(sequence).unwrap();
        let result = JoinedResultV1::new(
            job.clone(),
            agent.clone(),
            invocation.clone(),
            CodingResultOutcome::Completed,
            format!("durable decision {index}"),
            false,
        )
        .unwrap();
        let event = CodingEventV1::new(
            sequence,
            job.clone(),
            Some(agent),
            Some(invocation),
            CodingEventKind::StateChanged {
                state: JobState::Completed,
            },
        )
        .unwrap();
        fixture
            .store
            .record_coding_completion(event, &result)
            .await
            .unwrap();
    }
    NonZeroU64::new(sequence).unwrap()
}

pub(super) struct CaptureProvider {
    pub(super) endpoint: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl CaptureProvider {
    pub(super) fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let worker = thread::spawn(move || serve(listener, recorded, stopped));
        Self {
            endpoint,
            requests,
            stop,
            worker: Some(worker),
        }
    }

    pub(super) fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    pub(super) fn continuation_body(&self) -> Value {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .rev()
            .map(String::as_str)
            .find_map(context_body)
            .expect("resumed root request carries the continuation projection")
    }
}

impl Drop for CaptureProvider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

fn serve(listener: TcpListener, requests: Arc<Mutex<Vec<String>>>, stop: Arc<AtomicBool>) {
    let mut turns = BTreeMap::<String, usize>::new();
    // A safety net only: the worker also exits when `Drop` sets `stop`. A
    // direct-mode job does more owner work per run (W, merges, the ~2 s
    // readiness delta re-verification), so the net must outlast a loaded
    // full-suite run instead of cutting the provider off mid-test.
    let deadline = Instant::now() + Duration::from_secs(300);
    while !stop.load(Ordering::Acquire) && Instant::now() < deadline {
        let (mut stream, _) = match listener.accept() {
            Ok(stream) => stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
                continue;
            }
            Err(_) => break,
        };
        // macOS accept() inherits the listener's O_NONBLOCK, so reads would
        // return EAGAIN instantly; restore blocking mode. A connection that
        // stays silent across its read windows is a client that gave up
        // without closing; drain it and keep serving.
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let Some(request) = read_request(&mut stream) else {
            continue;
        };
        requests.lock().unwrap().push(request.clone());
        let actor = actor(&request);
        let turn = turns.entry(actor.clone()).or_default();
        send(&mut stream, &response(&actor, *turn));
        *turn += 1;
    }
}

fn context_body(request: &str) -> Option<Value> {
    serde_json::from_str::<Value>(request).ok()?["messages"]
        .as_array()?
        .iter()
        .filter_map(|message| message["content"].as_str())
        .find_map(|content| {
            content
                .split("source: Native job continuation (untrusted context; no grants)\n")
                .nth(1)?
                .split("\n<<<CONTEXT_BLOCK_END>>>")
                .next()
        })
        .and_then(|body| serde_json::from_str(body).ok())
}

fn actor(request: &str) -> String {
    // The request body is JSON, so the prompt's newline is the two-character
    // escape `\n` and whitespace-delimited parsing would swallow the rest of
    // the body into the actor (a fresh turn key per request). Scan the
    // agent-id shape directly instead, the way provider.rs does.
    request
        .split("UNTRUSTED CHILD TASK")
        .nth(1)
        .and_then(|task| task.split("write for ").nth(1))
        .and_then(|tail| {
            let suffix = tail.strip_prefix("child-")?;
            let end = suffix
                .find(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-')
                .unwrap_or(suffix.len());
            (end > 0).then_some(&tail[.."child-".len() + end])
        })
        .unwrap_or("parent")
        .to_owned()
}

fn response(actor: &str, turn: usize) -> String {
    // Direct mode shares one working checkout: every helper may change any
    // file, so the two pieces are disjoint by instruction, not by declared
    // paths, and one `helpers` call per helper waits for its own child.
    match (actor, turn) {
        ("parent", 0) => tool(
            "helpers",
            json!({"description":"write shared.txt","prompt":"write for child-a","subagent_type":"general","agent_id":"child-a"}),
        ),
        ("parent", 1) => tool(
            "helpers",
            json!({"description":"write rename-before.txt","prompt":"write for child-b","subagent_type":"general","agent_id":"child-b"}),
        ),
        ("parent", _) => answer("native parent finished"),
        (_, 0) => tool(
            "write",
            json!({"path":if actor == "child-a" { "shared.txt" } else { "rename-before.txt" },"content":format!("{actor}\n")}),
        ),
        _ => answer("child work complete"),
    }
}

fn read_request(stream: &mut TcpStream) -> Option<String> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    let mut stalling = 0usize;
    loop {
        let read = match stream.read(&mut buffer) {
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                stalling += 1;
                if stalling > 6 {
                    return None;
                }
                continue;
            }
            Err(error) => panic!("provider request read failed: {error}"),
        };
        stalling = 0;
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]);
            let length = headers.lines().find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            });
            if let Some(length) = length.filter(|length| bytes.len() >= end + 4 + length) {
                return Some(String::from_utf8(bytes[end + 4..end + 4 + length].to_vec()).unwrap());
            }
        }
    }
}

fn tool(name: &str, arguments: Value) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call","type":"function","function":{"name":name,"arguments":arguments.to_string()}}]},"finish_reason":"tool_calls"}]}),
        json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5}})
    )
}

fn answer(text: &str) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":"stop"}]}),
        json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5}})
    )
}

fn send(stream: &mut TcpStream, body: &str) {
    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    stream.flush().unwrap();
}
