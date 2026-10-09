//! The headless turn keeps an accepted task write when its next provider
//! request fails.

use super::*;
use saya_agent::ApprovalPolicy;
use saya_types::{SessionTask, SessionTaskList, TaskStatus};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

const TASK_CALL: &str = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-task\",\"function\":{\"name\":\"tasks_set\",\"arguments\":\"{\\\"tasks\\\":[{\\\"title\\\":\\\"keep the task\\\",\\\"status\\\":\\\"in_progress\\\"}]}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":1}}\n\ndata: [DONE]\n\n";

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

fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .expect("write provider response");
}

#[test]
fn headless_failure_keeps_a_task_write_from_an_earlier_tool_call() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local provider");
    let endpoint = format!(
        "http://{}/v1",
        listener.local_addr().expect("provider address")
    );
    listener
        .set_nonblocking(true)
        .expect("provider listener becomes nonblocking");
    let provider = thread::spawn(move || {
        let mut request_count = 0;
        let mut accept_request = || -> Result<TcpStream, String> {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        stream
                            .set_nonblocking(false)
                            .expect("accepted provider stream becomes blocking");
                        request_count += 1;
                        return Ok(stream);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if std::time::Instant::now() >= deadline {
                            return Err("provider stub received no request".to_owned());
                        }
                        thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(error) => return Err(error.to_string()),
                }
            }
        };
        let mut tool_request = accept_request().expect("accept tasks_set request");
        let _tool_body = read_request(&mut tool_request);
        respond(&mut tool_request, "200 OK", TASK_CALL);
        let mut answer_request = accept_request().expect("accept failed follow-up request");
        let answer_body = read_request(&mut answer_request);
        let request: serde_json::Value =
            serde_json::from_str(&answer_body).expect("follow-up request is JSON");
        assert!(
            request["messages"].as_array().is_some_and(|messages| {
                messages.iter().any(|message| {
                    message["role"] == "tool" && message["tool_call_id"] == "call-task"
                })
            }),
            "follow-up includes the tasks_set result"
        );
        respond(
            &mut answer_request,
            "500 Internal Server Error",
            "provider failed",
        );
        request_count
    });
    let root = std::env::temp_dir().join(format!(
        "saya-headless-task-sync-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("the task-sync root");
    let mut runtime = super::session_headless_receipt_tests::receipt_runtime();
    runtime.resolved.ai.provider = saya_config::AiProvider::OpenaiCompatible;
    runtime.resolved.ai.base_url = Some(endpoint);
    runtime.resolved.ai.retry_delays_ms.clear();
    let store = FsSessionStore::new(root.join("sessions"));
    let state_db = SqliteStateStore::new(std::path::PathBuf::new());
    let mut state = SessionState::new("headless-task-sync", None, "test-model");
    state.provider = "openai_compatible".into();
    let mut session = SessionRuntime::acquire(
        &runtime,
        Some(&root),
        true,
        None,
        "headless-task-sync",
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

    let outcome = handle_line_verbatim("hello", &mut ctx).expect("the turn settles");
    drop(ctx);
    assert!(matches!(outcome, TurnOutcome::Errored));
    let request_count = provider.join().expect("provider requests settle");
    eprintln!("local provider stub received {request_count} requests");
    assert_eq!(
        request_count, 2,
        "the stub receives the task call and failed follow-up"
    );
    assert_eq!(
        state.task_list,
        SessionTaskList::new(vec![
            SessionTask::new("keep the task", TaskStatus::InProgress).expect("valid task"),
        ])
        .expect("valid task list")
    );
    let _ = std::fs::remove_dir_all(root);
}
