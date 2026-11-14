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

/// An SSE body that ends before its terminator, having delivered nothing: the
/// retry path owns this cut (a flaky gateway recovers on the backoff), so the
/// modes that guard it — 60, 61, 65, 104, 105 — must deliver nothing. It
/// delivers nothing on purpose: a stream end *after* content is a stream cut,
/// which continues instead of retrying (mode 109 is that cut).
const CUT_STREAM: &str = "data: {\"choices\":[{\"delta\":{}}]}\n\n";

/// Mode 111's bounded held-child hold. The gate releases the moment the
/// sibling's first turn arrives, so this only bounds the wait when the second
/// never arrives: it must outlast two staged-and-admitted `helpers` calls,
/// which is seconds, not milliseconds.
const HELD_RELEASE: Duration = Duration::from_secs(10);

/// The stream a mode-109 child receives: a long reasoning flow, then the
/// connection closes with no terminator and no usage — the gateway cut seen
/// in practice. Reasoning is discarded, so this flow proves the stream
/// moved without sizing a later output ceiling.
const CUT_REASONING_DELTAS: usize = 40;
const CUT_REASONING_DELTA_BYTES: usize = 300;

fn reasoning_cut_stream() -> String {
    let text = "x".repeat(CUT_REASONING_DELTA_BYTES);
    let mut body = String::new();
    for _ in 0..CUT_REASONING_DELTAS {
        body.push_str("data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"");
        body.push_str(&text);
        body.push_str("\"}}]}\n\n");
    }
    body
}

impl Provider {
    pub(crate) fn script(injection: bool, mode: u8) -> Self {
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
            let mut cuts_left = match mode {
                60 => 1,
                61 | 65 | 104 | 105 => 2,
                _ => 0,
            };
            let mut pending = Vec::<(TcpStream, String)>::new();
            // Mode 109: the first child body, so every *identical* retry is cut
            // too. The identical request is what the loop re-sent; the
            // continuation differs, so it is served and the child finishes.
            let mut first_child_body: Option<String> = None;
            // Mode 111's held-child gate: when the first child's turn was held
            // and no sibling arrived, the hold must expire so a job that
            // refuses the second helper fails on a timeout instead of hanging.
            let mut held_at: Option<Instant> = None;
            // A safety net only: the worker also exits when `Drop` sets
            // `stop`. Sequenced-child jobs (for example 24 dependent children)
            // run far longer than any fixed bound, so the net must outlast
            // them instead of cutting the provider off mid-test.
            let deadline = Instant::now() + Duration::from_secs(600);
            while !stopped.load(Ordering::Acquire) && Instant::now() < deadline {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if matches!(mode, 48 | 88 | 93 | 94 | 105) && held.load(Ordering::Acquire) {
                            for (mut stream, response) in pending.drain(..) {
                                send(&mut stream, &response);
                            }
                        }
                        // Mode 111's bounded hold: a refused second helper
                        // leaves the first held forever, so release it once the
                        // hold expires and let the run fail on its assertions.
                        if mode == 111 && held_at.is_some_and(|at| at.elapsed() >= HELD_RELEASE) {
                            held_at = None;
                            for (mut stream, response) in pending.drain(..) {
                                send(&mut stream, &response);
                            }
                        }
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
                if cuts_left > 0 && actor == "child-one" {
                    // The stream ends before its terminator and the client
                    // retries the same turn, so it is not a script turn.
                    cuts_left -= 1;
                    *stage = current;
                    send(&mut stream, CUT_STREAM);
                    continue;
                }
                if mode == 109 && actor == "child-one" {
                    let cut = match &first_child_body {
                        Some(first) => *first == body,
                        None => {
                            first_child_body = Some(body.clone());
                            true
                        }
                    };
                    if cut {
                        // Cut before the terminator, so the request is not a
                        // script turn either.
                        *stage = current;
                        send(&mut stream, &reasoning_cut_stream());
                        continue;
                    }
                }
                let response = response(actor, current, injection, mode, &body);
                if mode == 4 {
                    pending.push((stream, response));
                    continue;
                }
                // The readiness-facts scenario queues the second status read
                // instead of blocking on it: the unawaited child must keep
                // being served while the test waits for it to fail.
                if mode == 105 && actor == "parent" && current == 2 {
                    pending.push((stream, response));
                    continue;
                }
                // Every mode dispatches its helpers through synchronous
                // `helpers` calls, so a child's first turn must be served
                // at once or the root's call would block forever. The held-child
                // modes are the exception: their child's first turn waits in
                // `pending` until the sibling's turn arrives (mode 111) or the
                // test releases the gate.
                if actor != "parent"
                    && current == 0
                    && matches!(mode, 48 | 88 | 93 | 94 | 105 | 111)
                {
                    pending.push((stream, response));
                    if mode == 111 && held_at.is_none() {
                        held_at = Some(Instant::now());
                    }
                    if pending.len() == 2 {
                        overlapped.store(true, Ordering::Release);
                        held_at = None;
                        for (mut stream, response) in pending.drain(..) {
                            send(&mut stream, &response);
                        }
                    }
                } else {
                    // The paired parent's final answer stays unsent until the
                    // test opens the gate (or the bounded hold expires), so
                    // the run cannot complete before the test has read the
                    // captured paired status result.
                    if mode == 6 && actor == "parent" && current >= 3 {
                        hold_for_status_read(&held);
                    }
                    if mode == 31 && actor != "parent" && current == 2 {
                        hold_for_status_read(&held);
                    }
                    if mode == 35 && actor != "parent" && current == 0 {
                        hold_for_status_read(&held);
                    }
                    // The readiness-cache scenarios hold the call under test
                    // (the preparation, or the final answer) until the test has
                    // written into the working checkout behind the cached
                    // status reads.
                    if matches!(mode, 99 | 100) && actor == "parent" && current == 2 {
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
