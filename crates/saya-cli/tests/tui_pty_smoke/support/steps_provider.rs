/// The script a step-line test needs: one turn that runs a shell command, and
/// the answer that follows it — the shape of a real turn, with nothing reaching
/// a network.
struct StepsProvider {
    address: String,
    requests: Arc<std::sync::Mutex<u64>>,
    stop: std::sync::mpsc::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl StepsProvider {
    /// The command the scripted turn runs: the line the transcript paints, and
    /// the words a test waits for.
    const COMMAND: &'static str = "echo hello from the steps turn";

    /// The words the turn answers with once the command has run.
    const ANSWER: &'static str = "the command printed its line";

    fn spawn() -> Self {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("the steps provider binds");
        let address = format!("http://{}", listener.local_addr().expect("its address"));
        listener
            .set_nonblocking(true)
            .expect("the listener becomes nonblocking");
        let requests = Arc::new(std::sync::Mutex::new(0_u64));
        let counted = Arc::clone(&requests);
        let (stop, stopped) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            loop {
                let mut stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if stopped.try_recv().is_ok() {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(_) => return,
                };
                if !answer_steps(&mut stream, &counted) {
                    return;
                }
            }
        });
        Self {
            address,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    /// The base URL the launch is pointed at.
    fn base_url(&self) -> String {
        format!("{}/v1", self.address)
    }

    /// How many requests reached the provider so far.
    fn requests(&self) -> u64 {
        *self.requests.lock().unwrap_or_else(|error| error.into_inner())
    }
}

impl Drop for StepsProvider {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Reads one request and answers it: the first is the shell call, every one
/// after it the turn's answer.
fn answer_steps(stream: &mut std::net::TcpStream, counted: &Arc<std::sync::Mutex<u64>>) -> bool {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    if drain_request(stream).is_empty() {
        return true;
    }
    let turn = {
        let mut count = counted.lock().unwrap_or_else(|error| error.into_inner());
        let turn = *count;
        *count += 1;
        turn
    };
    let body = match turn {
        0 => tool_call("shell", serde_json::json!({ "command": StepsProvider::COMMAND })),
        _ => streamed_answer(StepsProvider::ANSWER),
    };
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = write!(stream, "{head}{body}");
    let _ = stream.flush();
    true
}

/// One tool call, as the compatible provider streams it.
fn tool_call(name: &str, arguments: serde_json::Value) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        serde_json::json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":format!("call-{name}"),"type":"function","function":{"name":name,"arguments":arguments.to_string()}}]},"finish_reason":"tool_calls"}]}),
        serde_json::json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5}})
    )
}

/// One answer, streamed as the words it ends in.
fn streamed_answer(answer: &str) -> String {
    let mut body = String::new();
    for word in answer.split_inclusive(' ') {
        let delta = serde_json::json!({"choices": [{"delta": {"content": word}}]});
        body.push_str(&format!("data: {delta}\n\n"));
    }
    body.push_str(&format!(
        "data: {}\n\ndata: [DONE]\n\n",
        serde_json::json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5}})
    ));
    body
}
