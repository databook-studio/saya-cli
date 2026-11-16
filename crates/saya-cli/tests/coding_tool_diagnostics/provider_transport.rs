//! Accept-loop transport for the diagnostics provider: reads requests over
//! TCP, tracks turns per actor, and dispatches each to the owned script.
//! Actor routing keys on the child-task marker the harness brief adds and on
//! this script's own child ids, so parent and child bodies never collide.

use super::{Provider, script::response};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

impl Provider {
    pub(super) fn script(mode: u8) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&bodies);
        let responses = Arc::new(Mutex::new(Vec::new()));
        let sent = Arc::clone(&responses);
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
                // macOS accept() inherits the listener's O_NONBLOCK, so the
                // read timeout would be ignored and reads would return
                // EAGAIN instantly; restore blocking mode. A connection that
                // stays silent across its read windows is a client that gave
                // up without closing; drain it and keep serving.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let Some(body) = read_request(&mut stream) else {
                    continue;
                };
                captured.lock().unwrap().push(body.clone());
                let actor = actor_of(&body);
                let stage = turns.entry(actor.to_owned()).or_default();
                let current = *stage;
                *stage += 1;
                let response = response(actor, current, mode);
                send(&mut stream, &response);
                sent.lock().unwrap().push(response);
            }
        });
        Self {
            endpoint,
            bodies,
            responses,
            stop,
            worker: Some(worker),
        }
    }
}

fn actor_of(body: &str) -> &str {
    if !body.contains("UNTRUSTED CHILD TASK") {
        return "parent";
    }
    if body.contains("child-c1") {
        "child-c1"
    } else if body.contains("child-c2") {
        "child-c2"
    } else {
        "child"
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
