//! The session task list is re-rendered into a request only on the turn after
//! it changed. A turn whose list is unchanged carries no `session-tasks`
//! block: the model already has the state, from the block it was sent and
//! from its own `checklist` call result.

use super::*;
use saya_agent::ApprovalPolicy;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;

/// The block's own source line — what a request carries when it renders the
/// list, and what an unchanged turn must not carry.
const TASKS_MARKER: &str = "source: session-tasks";

/// One `checklist` call carrying a two-task list.
const TASK_CALL: &str = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-task\",\"function\":{\"name\":\"checklist\",\"arguments\":\"{\\\"tasks\\\":[{\\\"title\\\":\\\"parse records\\\",\\\"status\\\":\\\"in_progress\\\"},{\\\"title\\\":\\\"write the report\\\",\\\"status\\\":\\\"pending\\\"}]}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":1}}\n\ndata: [DONE]\n\n";

const ANSWER: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"done.\"},\"finish_reason\":\"stop\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":1}}\n\ndata: [DONE]\n\n";

fn read_request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let (header_end, content_length) = loop {
        let read = stream.read(&mut buffer).expect("read request headers");
        assert_ne!(read, 0, "request ended before headers");
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let headers = std::str::from_utf8(&bytes[..end]).expect("request headers are UTF-8");
            let length = headers
                .lines()
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .map(|(_, value)| value.trim().parse::<usize>().expect("content length"))
                })
                .expect("request has content length");
            break (end + 4, length);
        }
    };
    while bytes.len() < header_end + content_length {
        let read = stream.read(&mut buffer).expect("read request body");
        assert_ne!(read, 0, "request ended before body");
        bytes.extend_from_slice(&buffer[..read]);
    }
    String::from_utf8(bytes[header_end..header_end + content_length].to_vec())
        .expect("request body is UTF-8")
}

fn respond(stream: &mut TcpStream, body: &str) {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .expect("write provider response");
}

/// The `session-tasks` blocks in a captured request body, in message order.
fn blocks(body: &str) -> Vec<String> {
    let request: serde_json::Value =
        serde_json::from_str(body).expect("the captured request is JSON");
    request["messages"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|message| message["content"].as_str())
        .filter(|content| content.contains(TASKS_MARKER))
        .map(str::to_owned)
        .collect()
}

/// Serves every request the session makes until `stop` is set: the first
/// request of the session is answered with a `checklist` call, everything
/// after it with a plain answer.
fn serve(listener: TcpListener, stop: Arc<AtomicBool>) -> Vec<String> {
    listener
        .set_nonblocking(true)
        .expect("the stub listener becomes nonblocking");
    let mut bodies = Vec::new();
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream
                    .set_nonblocking(false)
                    .expect("the accepted stream becomes blocking");
                let body = read_request(&mut stream);
                let first = bodies.is_empty();
                bodies.push(body);
                respond(&mut stream, if first { TASK_CALL } else { ANSWER });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(error) => panic!("provider stub accept failed: {error}"),
        }
    }
    bodies
}

#[test]
fn an_unchanged_task_list_rides_one_turn_only() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local provider");
    let endpoint = format!(
        "http://{}/v1",
        listener.local_addr().expect("provider address")
    );
    let stop = Arc::new(AtomicBool::new(false));
    let provider = {
        let stop = Arc::clone(&stop);
        thread::spawn(move || serve(listener, stop))
    };
    let root = std::env::temp_dir().join(format!(
        "saya-headless-checklist-delta-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("the checklist-delta root");
    let mut runtime = super::session_headless_receipt_tests::receipt_runtime();
    runtime.resolved.ai.provider = saya_config::AiProvider::OpenaiCompatible;
    runtime.resolved.ai.base_url = Some(endpoint);
    runtime.resolved.ai.retry_delays_ms.clear();
    let store = FsSessionStore::new(root.join("sessions"));
    let state_db = SqliteStateStore::new(std::path::PathBuf::new());
    let mut state = SessionState::new("headless-checklist-delta", None, "test-model");
    state.provider = "openai_compatible".into();
    let mut session = SessionRuntime::acquire(
        &runtime,
        Some(&root),
        true,
        None,
        "headless-checklist-delta",
        ApprovalPolicy::Ask,
        &root.join("session-dirs"),
    )
    .expect("the session runtime acquires");
    let mut ctx = TurnContext {
        state: &mut state,
        runtime: std::sync::Arc::new(runtime),
        store: &store,
        state_db: &state_db,
        format: RenderFormat::Text,
        terminal: false,
        session: &mut session,
        trusted_echo: None,
        last_turn: None,
        save_refused: false,
        launch_options: GlobalOptions::default(),
    };

    // Turn 1 writes the list; turns 2 and 3 leave it alone.
    for line in ["track the work", "next", "again"] {
        let outcome = handle_line_verbatim(line, &mut ctx).expect("the turn settles");
        assert!(
            matches!(outcome, TurnOutcome::Completed),
            "{line} did not complete"
        );
    }
    drop(ctx);
    stop.store(true, Ordering::Release);
    let bodies = provider.join().expect("the provider stub settles");
    let _ = std::fs::remove_dir_all(root);

    assert_eq!(
        bodies.len(),
        4,
        "three turns: the checklist call and its follow-up, then one request per turn"
    );
    let rendered: Vec<Vec<String>> = bodies.iter().map(|body| blocks(body)).collect();
    assert!(
        rendered[0].is_empty() && rendered[1].is_empty(),
        "the turn that wrote the list never carried it: {rendered:#?}"
    );
    assert_eq!(
        rendered[2].len(),
        1,
        "the turn after the change carries the list once: {rendered:#?}"
    );
    assert!(
        rendered[2][0].contains("parse records"),
        "the block carries the stored list: {}",
        rendered[2][0]
    );
    assert!(
        rendered[3].is_empty(),
        "a turn whose list did not change carries nothing: {rendered:#?}"
    );

    // The cached prefix is untouched: the block rides the current user turn,
    // so dropping it leaves the system message and the replayed history as
    // they were.
    let unchanged: serde_json::Value =
        serde_json::from_str(&bodies[2]).expect("the carrying request is JSON");
    let dropped: serde_json::Value =
        serde_json::from_str(&bodies[3]).expect("the dropping request is JSON");
    assert_eq!(
        unchanged["messages"][0], dropped["messages"][0],
        "the system message is byte-identical whether or not the turn carries the list"
    );
    let carried = unchanged["messages"].as_array().unwrap();
    let after = dropped["messages"].as_array().unwrap();
    assert_eq!(
        &after[..carried.len() - 1],
        &carried[..carried.len() - 1],
        "the history the carrying turn replayed is replayed unchanged by the turn that drops the block"
    );
}
