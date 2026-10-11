use super::DEADLINE;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub struct Provider {
    endpoint: String,
    child_actors: Arc<Mutex<BTreeSet<String>>>,
    finished_actors: Arc<Mutex<BTreeSet<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

/// The read-loop turn pace: slow enough that the job's bounded logical
/// request budget outlives the operator's control sequence, fast enough that
/// child-b is demonstrably still running when cancel-child lands.
const READ_LOOP_PACE: Duration = Duration::from_millis(60);

pub fn config(provider: &Provider) -> String {
    format!(
        "[ai]\nprovider = 'openai_compatible'\nbase_url = '{}'\nmodel = 'pty-fixture'\n\n[coding]\nenabled = true\n\n[jobs.runner]\nprogram_dir = '/bin'\nallow = ['sleep']\n",
        provider.endpoint()
    )
}

impl Provider {
    pub fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let child_actors = Arc::new(Mutex::new(BTreeSet::new()));
        let finished_actors = Arc::new(Mutex::new(BTreeSet::new()));
        let stopped = Arc::clone(&stop);
        let actors = Arc::clone(&child_actors);
        let finished = Arc::clone(&finished_actors);
        let worker = thread::spawn(move || serve(listener, stopped, actors, finished));
        Self {
            endpoint,
            child_actors,
            finished_actors,
            stop,
            worker: Some(worker),
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn wait_for_children(&self, minimum: usize, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.child_actors.lock().unwrap().len() >= minimum {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("provider did not receive {minimum} child requests");
    }

    pub fn wait_for_child_finished(&self, actor: &str, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.finished_actors.lock().unwrap().contains(actor) {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("provider did not receive a finishing turn for {actor}");
    }

    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let deadline = Instant::now() + Duration::from_secs(4);
            while !worker.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            if !worker.is_finished() {
                self.worker = Some(worker);
                panic!("provider worker did not finish within its read deadline");
            }
            let _ = worker.join();
        }
    }
}

impl Drop for Provider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.take();
    }
}

fn serve(
    listener: TcpListener,
    stop: Arc<AtomicBool>,
    child_actors: Arc<Mutex<BTreeSet<String>>>,
    finished_actors: Arc<Mutex<BTreeSet<String>>>,
) {
    let deadline = Instant::now() + DEADLINE;
    let mut parent_turn = 0;
    let mut child_turns = BTreeMap::<String, usize>::new();
    while !stop.load(Ordering::Acquire) && Instant::now() < deadline {
        let (mut stream, _) = match listener.accept() {
            Ok(pair) => pair,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
                continue;
            }
            Err(_) => break,
        };
        // macOS hands out sockets that inherit the listener's O_NONBLOCK; the
        // request read below must block instead of racing the writer.
        stream.set_nonblocking(false).unwrap();
        let request = read_request(&mut stream);
        let body = if request.contains("UNTRUSTED CHILD TASK") {
            let actor = actor(&request);
            child_actors.lock().unwrap().insert(actor.clone());
            let turn = child_turns.entry(actor.clone()).or_default();
            let paced = *turn >= 2 && actor != "child-a";
            let body = child_response(&actor, *turn);
            *turn += 1;
            if *turn >= 3 {
                finished_actors.lock().unwrap().insert(actor.clone());
            }
            if paced {
                // The read loop only keeps child-b alive for the operator's
                // controls; pacing it leaves the job's bounded request budget
                // to the real controls instead of a hot spin.
                thread::sleep(READ_LOOP_PACE);
            }
            body
        } else {
            let response = parent_response(parent_turn);
            parent_turn += 1;
            response
        };
        send(&mut stream, &body);
    }
}

fn parent_response(turn: usize) -> String {
    match turn {
        0 => tool(
            "delegate",
            json!({"tasks":[
                {"agent_id":"child-a","task":"write for child-a","write_paths":["a.txt"]},
                {"agent_id":"child-b","task":"write for child-b","write_paths":["b.txt"]}
            ]}),
        ),
        1 => tool("await_children", json!({})),
        _ => answer("native parent finished after joining children"),
    }
}

fn actor(request: &str) -> String {
    request
        .split("write for ")
        .nth(1)
        .map(|tail| {
            // The body quotes the delegated task in several serializations
            // (`"task":"write for child-a"}`, history echoes, context
            // blocks); only the agent id itself is stable — stop at the
            // first character that cannot belong to an id, never at
            // whitespace, or the JSON that follows leaks into the key.
            tail.chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect()
        })
        .unwrap_or_else(|| "child".to_owned())
}

/// Turn 0 starts one supervised `sleep` (child-a's finishes in 2s, child-b's
/// holds 20s so it outlives the operator's controls). From turn 1 the child
/// reads its OWN command id — child-b keeps reading forever so it is still
/// running when the test cancels it, while child-a answers from turn 2:
/// its turn-1 read happens after the pause settled its command, so it
/// exercises the refusal-as-data path on the way to its answer.
fn child_response(actor: &str, turn: usize) -> String {
    let command_id = if actor == "child-a" { 1 } else { 2 };
    if turn == 0 {
        tool(
            "coding_command_start",
            json!({"program":"sleep","argv":[if actor == "child-a" { "2" } else { "20" }],"timeout_seconds":30}),
        )
    } else if actor == "child-a" && turn >= 2 {
        answer(&format!("{actor} child work complete"))
    } else {
        tool(
            "coding_command_read",
            json!({"command_id":command_id,"stderr":false}),
        )
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    loop {
        let count = match stream.read(&mut buffer) {
            Ok(0) => return String::new(),
            Ok(count) => count,
            Err(_) => return String::new(),
        };
        bytes.extend_from_slice(&buffer[..count]);
        assert!(bytes.len() <= 1024 * 1024, "provider request is bounded");
        if let Some(head) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..head]);
            let length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|value| value.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            if length <= 128 * 1024 && bytes.len() >= head + 4 + length {
                return String::from_utf8_lossy(&bytes[head + 4..head + 4 + length]).into_owned();
            }
        }
    }
}

fn send(stream: &mut TcpStream, body: &str) {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    stream.flush().unwrap();
}

fn tool(name: &str, arguments: Value) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":format!("call-{name}"),"type":"function","function":{"name":name,"arguments":arguments.to_string()}}]},"finish_reason":"tool_calls"}]}),
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
