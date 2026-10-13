//! Accept-loop transport for the scripted provider: reads requests over TCP,
//! tracks turns per actor, and dispatches each to the per-mode script.

use super::{Provider, script::response};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

impl Provider {
    pub(super) fn script(injection: bool, mode: u8) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let calls = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&calls);
        let overlap = Arc::new(AtomicBool::new(false));
        let overlapped = Arc::clone(&overlap);
        let saw_injection = Arc::new(AtomicBool::new(false));
        let injected = Arc::clone(&saw_injection);
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&bodies);
        let status_gate = Arc::new(AtomicBool::new(false));
        let held = Arc::clone(&status_gate);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            let mut turns = BTreeMap::<String, usize>::new();
            let mut pending = Vec::new();
            // A safety net only: the worker also exits when `Drop` sets
            // `stop`. Sequenced-child jobs (for example 24 dependent children)
            // run far longer than any fixed bound, so the net must outlast
            // them instead of cutting the provider off mid-test.
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
                // macOS hands out sockets that inherit the listener's O_NONBLOCK;
                // the request read below must block instead of racing the writer.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                // macOS accept() inherits the listener's O_NONBLOCK, so the
                // read timeout would be ignored and reads would return
                // EAGAIN instantly; restore blocking mode. A connection that
                // stays silent across its read windows is a client that gave
                // up without closing; drain it and keep serving.
                stream.set_nonblocking(false).unwrap();
                let Some(body) = read_request(&mut stream) else {
                    continue;
                };
                count.fetch_add(1, Ordering::AcqRel);
                captured.lock().unwrap().push(body.clone());
                if body.contains("INJECTED ROOT AUTHORITY") {
                    injected.store(true, Ordering::Release);
                }
                let actor = body
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
                    .unwrap_or("parent");
                let stage = turns.entry(actor.into()).or_default();
                let current = *stage;
                *stage += 1;
                let response = response(actor, current, injection, mode);
                if mode == 4 {
                    pending.push((stream, response));
                    continue;
                }
                if actor != "parent"
                    && current == 0
                    && mode != 3
                    && mode != 5
                    && mode != 7
                    && mode != 8
                {
                    pending.push((stream, response));
                    if pending.len() == 2 {
                        overlapped.store(true, Ordering::Release);
                        for (mut stream, response) in pending.drain(..) {
                            send(&mut stream, &response);
                        }
                    }
                } else {
                    // The parity parent's final answer stays unsent until the
                    // test opens the gate (or the bounded hold expires), so
                    // the run cannot complete before the test has read the
                    // captured coding_status result.
                    if mode == 6 && actor == "parent" && current >= 3 {
                        hold_for_status_read(&held);
                    }
                    send(&mut stream, &response);
                }
            }
        });
        Self {
            endpoint,
            calls,
            overlap,
            saw_injection,
            bodies,
            status_gate,
            stop,
            worker: Some(worker),
        }
    }
}
fn read_request(stream: &mut TcpStream) -> Option<String> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    let mut stalling = 0usize;
    loop {
        let n = match stream.read(&mut buffer) {
            Ok(n) => n,
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
fn send(stream: &mut TcpStream, body: &str) {
    write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
    stream.flush().unwrap();
}
fn hold_for_status_read(gate: &AtomicBool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !gate.load(Ordering::Acquire) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
}
