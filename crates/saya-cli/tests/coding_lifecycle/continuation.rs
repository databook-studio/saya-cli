use super::{provider::Provider, support::Fixture};
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingError, CodingJobRequest};
use saya_types::{JobId, ManagedCheckoutId};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::Read,
    io::Write,
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[tokio::test]
async fn continuation_context_compacts_large_history_without_losing_durable_identities() {
    let provider = CaptureProvider::new();
    let fixture = Fixture::with_endpoint("continuation-context", Some(&provider.endpoint)).await;
    let request = CodingJobRequest::new(
        JobId::parse("continuation-context").unwrap(),
        "Make dependent native child changes".into(),
        vec![
            "shared.txt".into(),
            "rename-before.txt".into(),
            "rename-after.txt".into(),
        ],
        vec![],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    job.run(&NoopEventSink).await.unwrap();
    let completion = fixture
        .store
        .list_coding_completion_history(job.job_id(), None, 32)
        .await
        .unwrap()
        .items
        .into_iter()
        .next()
        .unwrap();
    assert!(job.accept_finding(completion.sequence).await.unwrap());
    append_large_pending_tasks(&fixture, job.job_id());

    let approval = CodingAuthorization::scoped(
        &fixture
            .application
            .propose_resume(job.job_id())
            .await
            .unwrap(),
    )
    .unwrap();
    let resumed = fixture
        .application
        .resume(job.job_id(), &approval, CancellationToken::new())
        .await
        .unwrap();
    let candidate = ManagedCheckoutId::parse("continuation-conflict").unwrap();
    assert!(matches!(
        resumed.review(&candidate).await,
        Err(CodingError::ConflictRetained { .. })
    ));
    let conflict = fixture
        .application
        .conflicts(resumed.job_id(), &candidate)
        .await
        .unwrap();
    let output = resumed.run(&NoopEventSink).await.unwrap();
    assert!(!output.truncated);
    assert!(provider.request_count() > 0);

    let body = provider.continuation_body();
    assert_eq!(body["version"], 1);
    assert_eq!(body["authority"], "none");
    assert!(
        body["pending_tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["agent_id"] == "pending-04" && task["state"] == "not_started")
    );
    assert!(
        body["accepted_decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|decision| decision["sequence"] == completion.sequence.get())
    );
    assert_eq!(
        body["pending_conflicts"][0]["record_sha256"],
        conflict["provenance"]["record_sha256"]
    );
    assert!(body.to_string().len() <= 48 * 1024);
    assert!(body["omitted"]["task_history_bytes"].as_u64().unwrap() > 48 * 1024);
}

#[tokio::test]
async fn malformed_task_history_refuses_before_a_resumed_provider_request() {
    let provider = Provider::new(false);
    let fixture = Fixture::with_endpoint("continuation-malformed", Some(&provider.endpoint)).await;
    let request = CodingJobRequest::new(
        JobId::parse("continuation-malformed").unwrap(),
        "Make native child changes".into(),
        vec!["new-a.txt".into(), "new-b.txt".into()],
        vec![],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    job.run(&NoopEventSink).await.unwrap();
    std::fs::write(
        fixture
            .root
            .join("coding")
            .join(job.job_id().as_str())
            .join("tasks.jsonl"),
        b"not-json\n",
    )
    .unwrap();
    let approval = CodingAuthorization::scoped(
        &fixture
            .application
            .propose_resume(job.job_id())
            .await
            .unwrap(),
    )
    .unwrap();
    let resumed = fixture
        .application
        .resume(job.job_id(), &approval, CancellationToken::new())
        .await
        .unwrap();
    let calls = provider.calls.load(Ordering::Acquire);
    assert!(resumed.run(&NoopEventSink).await.is_err());
    assert_eq!(provider.calls.load(Ordering::Acquire), calls);
}

fn append_large_pending_tasks(fixture: &Fixture, job: &JobId) {
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

struct CaptureProvider {
    endpoint: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl CaptureProvider {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            let mut turns = BTreeMap::<String, usize>::new();
            let deadline = Instant::now() + Duration::from_secs(30);
            while !stopped.load(Ordering::Acquire) && Instant::now() < deadline {
                let (mut stream, _) = match listener.accept() {
                    Ok(stream) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(_) => break,
                };
                let request = read_request(&mut stream);
                recorded.lock().unwrap().push(request.clone());
                let actor = actor(&request);
                let turn = turns.entry(actor.clone()).or_default();
                let response = response(&actor, *turn);
                *turn += 1;
                send(&mut stream, &response);
            }
        });
        Self {
            endpoint,
            requests,
            stop,
            worker: Some(worker),
        }
    }

    fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    fn continuation_body(&self) -> Value {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find_map(|request| {
                let request: Value = serde_json::from_str(request).ok()?;
                request["messages"]
                    .as_array()?
                    .iter()
                    .filter_map(|message| message["content"].as_str())
                    .find_map(|content| {
                        content
                            .split(
                                "source: Native job continuation (untrusted context; no grants)\n",
                            )
                            .nth(1)?
                            .split("\n<<<CONTEXT_BLOCK_END>>>")
                            .next()
                    })
                    .and_then(|body| serde_json::from_str(body).ok())
            })
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

fn actor(request: &str) -> String {
    request
        .split("UNTRUSTED CHILD TASK")
        .nth(1)
        .and_then(|task| task.split("write for ").nth(1))
        .and_then(|tail| tail.split_whitespace().next())
        .unwrap_or("parent")
        .to_owned()
}

fn response(actor: &str, turn: usize) -> String {
    let content = match (actor, turn) {
        ("parent", 0) => json!({"tasks":[
            {"agent_id":"child-a","task":"write for child-a","write_paths":["shared.txt"]},
            {"agent_id":"child-b","task":"write for child-b","write_paths":["shared.txt"],"dependencies":["child-a"]}
        ]}),
        ("parent", 1) => json!({}),
        ("parent", _) => return answer("native parent finished"),
        (_, 0) => json!({"path":"shared.txt","content":format!("{actor}\n")}),
        _ => return answer("child work complete"),
    };
    let name = if actor == "parent" {
        if turn == 0 {
            "delegate"
        } else {
            "await_children"
        }
    } else {
        "workspace_write"
    };
    tool(name, content)
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let read = stream.read(&mut buffer).unwrap();
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]);
            let length = headers.lines().find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            });
            if let Some(length) = length.filter(|length| bytes.len() >= end + 4 + length) {
                return String::from_utf8(bytes[end + 4..end + 4 + length].to_vec()).unwrap();
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
