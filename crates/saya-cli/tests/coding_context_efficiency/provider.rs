use super::{PROVIDER_LIFETIME, script::Scenario};
use serde_json::Value;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub(super) struct Provider {
    pub endpoint: String,
    bodies: Arc<Mutex<Vec<Value>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Provider {
    pub fn new(scenario: Scenario) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&bodies);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            let mut turns = [0, 0];
            let deadline = Instant::now() + PROVIDER_LIFETIME;
            while !stopped.load(Ordering::Acquire) && Instant::now() < deadline {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                };
                stream.set_nonblocking(false).unwrap();
                // Load ceiling, not a product bound: a client that is starved of
                // CPU can take a while to flush one request body, and a timeout
                // here fails the fixture instead of the behavior under test.
                stream
                    .set_read_timeout(Some(Duration::from_secs(30)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(30)))
                    .unwrap();
                let body = read(&mut stream);
                let child = is_child(&body);
                captured.lock().unwrap().push(body);
                let actor = usize::from(child);
                let response = scenario.response(child, turns[actor]);
                turns[actor] += 1;
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
                stream.flush().unwrap();
            }
        });
        Self {
            endpoint,
            bodies,
            stop,
            worker: Some(worker),
        }
    }
    pub fn bodies(&self) -> Vec<Value> {
        self.bodies.lock().unwrap().clone()
    }
}
pub(super) fn is_child(body: &Value) -> bool {
    body["messages"].as_array().unwrap().iter().any(|message| {
        message["content"]
            .as_str()
            .is_some_and(|text| text.contains("UNTRUSTED CHILD TASK ("))
    })
}
fn read(stream: &mut TcpStream) -> Value {
    let mut bytes = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let count = stream.read(&mut buffer).unwrap();
        assert!(count > 0, "fixture request closed before complete body");
        bytes.extend_from_slice(&buffer[..count]);
        assert!(bytes.len() <= 8 * 1024 * 1024, "bounded fixture request");
        if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..index]);
            let length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|text| text.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            if bytes.len() >= index + 4 + length {
                return serde_json::from_slice(&bytes[index + 4..index + 4 + length]).unwrap();
            }
        }
    }
}
impl Drop for Provider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let result = worker.join();
            if !std::thread::panicking() {
                result.unwrap();
            }
        }
    }
}
