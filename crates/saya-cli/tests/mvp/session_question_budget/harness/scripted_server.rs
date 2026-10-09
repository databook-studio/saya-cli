#[cfg(unix)]
fn reply_finished(stream: &mut std::net::TcpStream, body: &str) {
    use std::io::Write;
    let done = format!("data: [{}]\n\n", "DONE");
    let full = format!("{body}{done}");
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{full}"
    )
    .unwrap();
    stream.flush().unwrap();
}

#[cfg(unix)]
pub(crate) struct MockServer {
    pub(crate) address: String,
    pub(crate) requests: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    /// The request bodies the mock received, in arrival order.
    pub(crate) bodies: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

#[cfg(unix)]
impl MockServer {
    pub(crate) fn join(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(unix)]
impl Drop for MockServer {
    fn drop(&mut self) {
        self.join();
    }
}

/// Spawns the mock provider thread: request `i` is answered by
/// `script[i]` (requests past the script get a harmless completion).
/// Clarify holds `delay` inside the thread before writing (source-held
/// ACTIVE time); HoldOpen writes one marker chunk and keeps the socket
/// open until `release` fires, so no new request can arrive meanwhile.
#[cfg(unix)]
pub(crate) fn spawn_script_server(
    listener: std::net::TcpListener,
    script: Vec<ScriptStep>,
    release: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> MockServer {
    use std::{io::Write, sync::atomic::Ordering, thread, time::Duration};

    let address = format!("http://{}", listener.local_addr().unwrap());
    let requests = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let server_count = requests.clone();
    let bodies = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let server_bodies = bodies.clone();
    let handle = thread::spawn(move || {
        loop {
            let Some(mut stream) = accept_provider(&listener) else {
                break;
            };
            let body = drain_provider_request(&mut stream);
            server_bodies.lock().unwrap().push(body);
            let index = server_count.fetch_add(1, Ordering::SeqCst);
            let step = script.get(index).cloned().unwrap_or(ScriptStep::Harmless);
            match step {
                ScriptStep::Clarify { delay } => {
                    thread::sleep(delay);
                    reply_finished(&mut stream, &clarification_body());
                }
                ScriptStep::Natural => reply_finished(&mut stream, &answer_body()),
                ScriptStep::NaturalWithUsage { input_tokens } => {
                    reply_finished(&mut stream, &answer_with_usage_body(input_tokens))
                }
                ScriptStep::NaturalNoUsage => reply_finished(&mut stream, &answer_no_usage_body()),
                ScriptStep::Summary(text) => reply_finished(&mut stream, &summary_body(&text)),
                ScriptStep::DelayedNatural { delay } => {
                    thread::sleep(delay);
                    reply_finished(&mut stream, &answer_body());
                }
                ScriptStep::Harmless => reply_finished(&mut stream, &harmless_body()),
                ScriptStep::HoldOpen { marker } => {
                    // A 429 establishment refusal: the loop backs off and
                    // retries the SAME answering request against the next
                    // script slot, so the run stays inside the answering
                    // source, held, until the remaining budget fires. (A
                    // 200 + content + DONE would settle the turn naturally
                    // instead of tripping the ceiling.) `marker` selects
                    // which unfinished chunk the retry serves: empty keeps
                    // the socket silent; non-empty writes one content chunk
                    // (already on screen, proving the reply turn runs)
                    // with no DONE frame, then holds the socket open.
                    if marker.is_empty() {
                        // Hold the socket silent: accept the request, say
                        // nothing, keep it open. The establishment (60s)
                        // and per-chunk idle (90s) timeouts never fire, so
                        // the runtime's own remaining-elapsed timer is what
                        // ends the reply turn — still ACTIVE elapsed time
                        // of the same question scope. (A 429 refusal is NOT
                        // usable here: a body on a 429 confuses reqwest's
                        // framing and the loop classes it as a not-retried
                        // provider failure instead of backing off.)
                        use std::time::Instant;
                        let held = Instant::now();
                        while !release.load(Ordering::SeqCst) {
                            if held.elapsed() > Duration::from_secs(25) {
                                break;
                            }
                            thread::sleep(Duration::from_millis(20));
                        }
                        return;
                    }
                    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n";
                    write!(stream, "{head}data: {marker}\n\n").unwrap();
                    stream.flush().unwrap();
                    use std::time::Instant;
                    let held = Instant::now();
                    let mut beats = 0_u32;
                    while !release.load(Ordering::SeqCst) {
                        if held.elapsed() > Duration::from_secs(25) {
                            break;
                        }
                        if beats.is_multiple_of(10) {
                            let _ = write!(stream, ": keep-alive\n\n");
                            let _ = stream.flush();
                        }
                        beats = beats.wrapping_add(1);
                        thread::sleep(Duration::from_millis(20));
                    }
                    return;
                }
            }
            drop(stream);
            // The latch rows script up to ten requests (fail, /compact
            // re-arm, /clear re-arm); bound runaway loops well past that.
            if index >= 16 {
                break;
            }
        }
    });
    MockServer {
        address,
        requests,
        bodies,
        handle: Some(handle),
    }
}
