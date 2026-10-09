#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestKind {
    Tool,
    Answer,
    Extraction,
}

/// The local source is deliberately sequential: the third observed request is
/// accepted only when it carries the extraction system marker. Its release is
/// the natural bounded completion of the ancillary stream, not a test seam in
/// the runtime.
fn timer_source(
    extraction_started: tokio::sync::oneshot::Sender<()>,
    release: mpsc::Receiver<()>,
    seen: Arc<Mutex<Vec<RequestKind>>>,
) -> (String, thread::JoinHandle<Result<(), String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind timer source");
    listener
        .set_nonblocking(true)
        .expect("timer source listener becomes nonblocking");
    let address = listener.local_addr().expect("timer source address");
    let handle = thread::spawn(move || {
        let mut extraction_started = Some(extraction_started);
        for expected in [
            RequestKind::Tool,
            RequestKind::Answer,
            RequestKind::Extraction,
        ] {
            let accept_deadline = std::time::Instant::now() + Duration::from_secs(4);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(accepted) => break accepted,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if release.try_recv().is_ok() {
                            return Err("timer source released before request".into());
                        }
                        if std::time::Instant::now() >= accept_deadline {
                            return Err("timer source timed out waiting for request".into());
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => return Err(error.to_string()),
                }
            };
            // Accepted sockets inherit O_NONBLOCK from a non-blocking listener on macOS.
            stream
                .set_nonblocking(false)
                .map_err(|error| error.to_string())?;
            let body = read_request(&mut stream)?;
            let kind = classify_request(&body)?;
            if kind != expected {
                return Err(format!("expected {expected:?}, received {kind:?}"));
            }
            seen.lock()
                .map_err(|_| "request log poisoned".to_string())?
                .push(kind);
            match kind {
                RequestKind::Tool => respond(&mut stream, TOOL_RESPONSE)?,
                RequestKind::Answer => respond(&mut stream, ANSWER_RESPONSE)?,
                RequestKind::Extraction => {
                    extraction_started
                        .take()
                        .expect("only one extraction source request")
                        .send(())
                        .map_err(|_| "runtime stopped before extraction observation".to_string())?;
                    release
                        .recv_timeout(Duration::from_secs(4))
                        .map_err(|_| "test did not release extraction".to_string())?;
                    respond(&mut stream, EXTRACTION_RESPONSE)?;
                }
            }
        }
        Ok(())
    });
    (format!("http://{address}/v1"), handle)
}

fn read_request(stream: &mut TcpStream) -> Result<String, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let (header_end, content_length) = loop {
        let read = stream
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("source request ended before headers".into());
        }
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let header = std::str::from_utf8(&bytes[..end]).map_err(|error| error.to_string())?;
            let length = header
                .lines()
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .map(|(_, value)| value.trim())
                })
                .ok_or_else(|| "source request omitted content length".to_string())?
                .parse::<usize>()
                .map_err(|error| error.to_string())?;
            break (end + 4, length);
        }
    };
    while bytes.len() < header_end + content_length {
        let read = stream
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("source request ended before body".into());
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    String::from_utf8(bytes[header_end..header_end + content_length].to_vec())
        .map_err(|error| error.to_string())
}

fn classify_request(body: &str) -> Result<RequestKind, String> {
    let request: serde_json::Value =
        serde_json::from_str(body).map_err(|error| error.to_string())?;
    let messages = request["messages"]
        .as_array()
        .ok_or_else(|| "source request omitted messages".to_string())?;
    let system = messages
        .first()
        .and_then(|message| message["content"].as_str())
        .ok_or_else(|| "source request omitted system text".to_string())?;
    if system.contains("precision schema knowledge extractor") {
        return Ok(RequestKind::Extraction);
    }
    let tools = request["tools"]
        .as_array()
        .ok_or_else(|| "answering request omitted tools".to_string())?;
    let has_sql = tools.iter().any(|tool| {
        tool["function"]["name"]
            .as_str()
            .is_some_and(|name| name == "bounded_sql_query")
    });
    if !has_sql {
        return Err("answering request did not advertise bounded_sql_query".into());
    }
    if messages.iter().any(|message| message["role"] == "tool") {
        Ok(RequestKind::Answer)
    } else {
        Ok(RequestKind::Tool)
    }
}

fn respond(stream: &mut TcpStream, body: &str) -> Result<(), String> {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|error| error.to_string())
}

async fn runtime_for_timer_source(root: &std::path::Path, base_url: String) -> RuntimeConfig {
    let database = root.join("timer.sqlite3");
    let seed = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&database)
            .create_if_missing(true),
    )
    .await
    .expect("open synthetic SQLite database");
    sqlx::query("CREATE TABLE orders (id INTEGER)")
        .execute(&seed)
        .await
        .expect("seed synthetic SQLite table");
    sqlx::query("INSERT INTO orders (id) VALUES (1)")
        .execute(&seed)
        .await
        .expect("seed synthetic SQLite row");
    seed.close().await;
    let profile = DatabaseProfile::Sqlite {
        path: database.display().to_string(),
        read_only: true,
    };
    let mut runtime = test_runtime(assisted_memory());
    runtime.resolved.profile_name = Some("analytics".into());
    runtime.resolved.profile = Some(profile.clone());
    runtime.resolved.ai = ResolvedAi {
        provider: AiProvider::OpenaiCompatible,
        model: "timer-test".into(),
        base_url: Some(base_url),
        api_key: None,
        allow_data_sharing: true,
        temperature: 0.0,
        timeout_seconds: 2,
        idle_timeout_seconds: 4,
        max_output_tokens: 128,
        max_output_tokens_is_default: false,
        context_byte_budget: 256 * 1024,
        context_window_tokens: None,
        show_thinking: false,
        compaction: saya_config::CompactionMode::Auto,
        retry_delays_ms: Vec::new(),
    };
    runtime.resolved.investigation_budgets = ResolvedInvestigationBudgets {
        logical_answering_requests: finite(8),
        requested_tool_calls: finite(8),
        elapsed_seconds: finite(1),
        known_reported_tokens: finite(100),
    };
    runtime.connections = ConnectionsFile {
        profiles: BTreeMap::from([("analytics".into(), profile)]),
    };
    runtime.connections_path = Some(root.join("connections.toml"));
    runtime.cache_scope = root.to_path_buf();
    runtime.investigations_root = root.join("investigations");
    runtime
}
