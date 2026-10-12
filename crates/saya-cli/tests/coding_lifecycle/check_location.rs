//! Real agent dispatch of the check tool must execute inside the child's own
//! owned checkout, never in the original repository. The shared `Provider`
//! never writes files before dispatching a check, so this file carries a
//! fixture-private provider that scripts each child to write its assigned
//! file first and only then dispatch `coding_check`; the receipt must then
//! carry the child's post-write source digest, which no other tree can
//! produce because only that checkout contains the assigned file.

use super::support::Fixture;
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingJobRequest};
use saya_harness::coding::integration::{DEFAULT_DELTA_DEADLINE, candidate_source_digest};
use saya_types::JobId;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

struct ScriptedProvider {
    endpoint: String,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl ScriptedProvider {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            let mut turns = BTreeMap::<String, usize>::new();
            let deadline = Instant::now() + Duration::from_secs(600);
            while !stopped.load(Ordering::Acquire) && Instant::now() < deadline {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(_) => break,
                };
                // macOS accept hands out sockets that inherit O_NONBLOCK; the
                // request read below must block instead of racing the writer.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let Some(body) = read_request(&mut stream) else {
                    continue;
                };
                let actor = actor_of(&body);
                let stage = turns.entry(actor.clone()).or_default();
                let current = *stage;
                *stage += 1;
                send(&mut stream, &response(&actor, current));
            }
        });
        Self {
            endpoint,
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for ScriptedProvider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

fn read_request(stream: &mut TcpStream) -> Option<String> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let n = match stream.read(&mut buffer) {
            Ok(n) => n,
            Err(_) => return None,
        };
        assert!(n > 0, "provider request closed early");
        bytes.extend_from_slice(&buffer[..n]);
        assert!(bytes.len() <= 1024 * 1024);
        if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..index]);
            let length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|value| value.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            if bytes.len() >= index + 4 + length {
                return Some(
                    String::from_utf8(bytes[index + 4..index + 4 + length].to_vec()).unwrap(),
                );
            }
        }
    }
}

fn actor_of(body: &str) -> String {
    // Mirrors provider.rs: the request body is JSON, so scan the agent-id
    // shape directly instead of whitespace-delimited parsing.
    body.split("UNTRUSTED CHILD TASK")
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

fn response(actor: &str, stage: usize) -> String {
    if actor == "parent" {
        return match stage {
            0 => tool(
                "delegate",
                json!({"tasks":[{"agent_id":"child-a","task":"write for child-a","write_paths":["new-a.txt"]},{"agent_id":"child-b","task":"write for child-b","write_paths":["new-b.txt"]}]}),
            ),
            1 => tool("await_children", json!({})),
            _ => answer("location parent finished after joining children"),
        };
    }
    match stage {
        0 => {
            let path = if actor == "child-a" {
                "new-a.txt"
            } else {
                "new-b.txt"
            };
            tool(
                "workspace_write",
                json!({"path":path,"content":format!("located {actor}\n")}),
            )
        }
        1 => tool("coding_check", json!({"profile":"lint"})),
        _ => answer("child check evidence obtained"),
    }
}

fn tool(name: &str, args: Value) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":format!("call-{name}"),"type":"function","function":{"name":name,"arguments":args.to_string()}}]},"finish_reason":"tool_calls"}]}),
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
    write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
    stream.flush().unwrap();
}

#[tokio::test]
async fn a_dispatched_check_runs_in_the_childs_owned_checkout_and_never_in_the_original_repository()
{
    let provider = ScriptedProvider::start();
    let fixture = Fixture::with_check("check-location", &provider.endpoint).await;
    let request = CodingJobRequest::new(
        JobId::parse("check-location").unwrap(),
        "Run checks where each child owns its files".into(),
        vec!["new-a.txt".into(), "new-b.txt".into()],
        vec!["false".into()],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(15), job.run(&NoopEventSink))
        .await
        .unwrap()
        .unwrap();

    let receipts = receipts(&fixture, "check-location", "checks-");
    assert_eq!(receipts.len(), 2, "each child dispatches exactly one check");
    let checkouts = fixture
        .store
        .list_managed_checkouts(job.job_id(), None, 10)
        .await
        .unwrap()
        .items;
    assert_eq!(checkouts.len(), 2);

    let mut digests = Vec::new();
    for checkout in &checkouts {
        let agent = checkout.ownership.agent_id().unwrap().to_string();
        let path = Path::new(checkout.ownership.managed_path().as_str());
        assert_ne!(path, fixture.repository, "a child never owns the original");
        let own_file = if agent == "child-a" {
            "new-a.txt"
        } else {
            "new-b.txt"
        };
        assert!(path.join(own_file).exists());
        let receipt = receipts
            .iter()
            .find(|receipt| {
                receipt["checkout_id"] == json!(checkout.ownership.managed_checkout_id())
            })
            .unwrap_or_else(|| panic!("no check receipt for the checkout of {agent}"));
        let digest =
            candidate_source_digest(path, &CancellationToken::new(), DEFAULT_DELTA_DEADLINE)
                .unwrap();
        assert_eq!(
            receipt["receipt"]["source_sha256"].as_str().unwrap(),
            digest.as_str(),
            "the check receipt must bind {agent}'s own checkout tree"
        );
        digests.push(digest);
    }
    assert_ne!(
        digests[0], digests[1],
        "children with different files cannot share a source digest"
    );
    for name in ["new-a.txt", "new-b.txt"] {
        assert!(
            !fixture.repository.join(name).exists(),
            "{name} must never appear in the original repository"
        );
    }
}

fn receipts(fixture: &Fixture, job: &str, prefix: &str) -> Vec<Value> {
    std::fs::read_dir(fixture.root.join("coding").join(job))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(prefix)
        })
        .flat_map(|path| {
            std::fs::read_to_string(path)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect::<Vec<Value>>()
        })
        .collect()
}
