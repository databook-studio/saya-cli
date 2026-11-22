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

/// One tool call the mock served, as the model asked for it. The goldens
/// record these rather than only the calls replayed in a later request:
/// a clarification call ends its turn and is not replayed, but the model
/// still made it.
#[derive(Clone)]
pub(crate) struct ServedCall {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) arguments: serde_json::Value,
}

pub(crate) struct MockServer {
    pub(crate) address: String,
    pub(crate) requests: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    /// The request bodies the mock received, in arrival order.
    pub(crate) bodies: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    /// The tool calls the mock served, in arrival order.
    pub(crate) served: std::sync::Arc<std::sync::Mutex<Vec<ServedCall>>>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl MockServer {
    pub(crate) fn join(&mut self) {
        // The test is done with the mock: wake the accept loop instead of
        // making the join wait out its bounded deadline.
        self.stop
            .store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

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
pub(crate) fn spawn_script_server(
    listener: std::net::TcpListener,
    script: Vec<ScriptStep>,
    release: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> MockServer {
    spawn_script_server_limit(listener, script, release, 16, std::time::Duration::from_secs(30))
}

/// The same mock with the request cap and the accept wait stated. A scenario's
/// latch rows script up to ten requests and the cap of sixteen covers them; the
/// perf harness scripts one request per measured turn, so it states its own cap
/// rather than the sixteen those rows bound — and a longer accept wait, because
/// a measure may sit idle for a minute before the turn it asks for.
pub(crate) fn spawn_script_server_limit(
    listener: std::net::TcpListener,
    script: Vec<ScriptStep>,
    release: std::sync::Arc<std::sync::atomic::AtomicBool>,
    last_index: usize,
    accept_wait: std::time::Duration,
) -> MockServer {
    use std::{io::Write, sync::atomic::Ordering, thread, time::Duration};

    let address = format!("http://{}", listener.local_addr().unwrap());
    let requests = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let server_count = requests.clone();
    let bodies = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let server_bodies = bodies.clone();
    let served = std::sync::Arc::new(std::sync::Mutex::new(Vec::<ServedCall>::new()));
    let server_served = served.clone();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let server_stop = stop.clone();
    let handle = thread::spawn(move || {
        loop {
            let Some(mut stream) = accept_provider(&listener, &server_stop, accept_wait)
            else {
                break;
            };
            let body = drain_provider_request(&mut stream);
            server_bodies.lock().unwrap().push(body);
            let index = server_count.fetch_add(1, Ordering::SeqCst);
            let step = script.get(index).cloned().unwrap_or(ScriptStep::Harmless);
            match step {
                ScriptStep::Clarify { delay } => {
                    thread::sleep(delay);
                    server_served.lock().unwrap().push(ServedCall {
                        id: tool_call_id(index),
                        name: "request_clarification".to_owned(),
                        arguments: clarification_arguments(),
                    });
                    reply_finished(&mut stream, &clarification_body(index));
                }
                ScriptStep::Natural => reply_finished(&mut stream, &answer_body()),
                ScriptStep::NaturalWithUsage { input_tokens } => {
                    reply_finished(&mut stream, &answer_with_usage_body(input_tokens))
                }
                ScriptStep::NaturalNoUsage => reply_finished(&mut stream, &answer_no_usage_body()),
                ScriptStep::Summary(text) => reply_finished(&mut stream, &summary_body(&text)),
                ScriptStep::DelayedSummary { text, delay } => {
                    thread::sleep(delay);
                    reply_finished(&mut stream, &summary_body(&text));
                }
                ScriptStep::StreamTokens { count, chunk_delay } => {
                    // The head goes out first, then one frame per chunk, each
                    // flushed: the reader sees a stream, not one body.
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                         Connection: close\r\n\r\n"
                    )
                    .unwrap();
                    stream.flush().unwrap();
                    for chunk in 0..count {
                        write!(stream, "{}", token_frame(chunk)).unwrap();
                        stream.flush().unwrap();
                        thread::sleep(chunk_delay);
                    }
                    write!(stream, "data: [DONE]\n\n").unwrap();
                    stream.flush().unwrap();
                }
                ScriptStep::Json(content) => {
                    reply_finished(&mut stream, &json_content_body(&content))
                }
                ScriptStep::DelayedNatural { delay } => {
                    thread::sleep(delay);
                    reply_finished(&mut stream, &answer_body());
                }
                ScriptStep::ToolCall { name, arguments } => {
                    server_served.lock().unwrap().push(ServedCall {
                        id: tool_call_id(index),
                        name: name.clone(),
                        arguments: arguments.clone(),
                    });
                    reply_finished(&mut stream, &tool_call_body(&name, &arguments, index));
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
                    //
                    // A release closes this one response and leaves the mock
                    // serving: a scenario releases the held request to drive
                    // the turn AFTER it (the next line of the same run) on the
                    // same script.
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
                    } else {
                        let head =
                            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n";
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
                    }
                }
            }
            drop(stream);
            // The latch rows script up to ten requests (fail, /compact
            // re-arm, /clear re-arm); bound runaway loops well past that.
            if index >= last_index {
                break;
            }
        }
    });
    MockServer {
        address,
        requests,
        bodies,
        served,
        stop,
        handle: Some(handle),
    }
}
