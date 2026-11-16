struct TestServer {
    stdin: Option<ChildStdin>,
    responses: Receiver<String>,
    child: Child,
}

impl TestServer {
    fn spawn(fixture: &Fixture, extra: &[&str]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_saya"))
            .args([
                "--config",
                fixture.config.to_str().unwrap(),
                "--connections",
                fixture.connections.to_str().unwrap(),
            ])
            .args(extra)
            .env("SAYA_STATE_DB", &fixture.state)
            .env("SAYA_INVESTIGATIONS_DIR", &fixture.investigations)
            // Hermetic config discovery: the user layer is this fixture's
            // home only, never the running machine's.
            .env("HOME", fixture.root.join("home"))
            .env("XDG_CONFIG_HOME", fixture.root.join("home").join(".config"))
            .env_remove("SAYA_CONFIG_HOME")
            .env_remove("XDG_DATA_HOME")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn the saya binary");
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let (sender, responses) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut buf = Vec::new();
            loop {
                buf.clear();
                // read_until keeps the terminating newline: the wire bound is
                // asserted on the reply line's exact bytes, newline included.
                match reader.read_until(b'\n', &mut buf) {
                    Ok(0) => break,
                    Ok(_) => {
                        let line = String::from_utf8_lossy(&buf).into_owned();
                        if sender.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            stdin: Some(stdin),
            responses,
            child,
        }
    }

    fn send(&mut self, line: &str) {
        let stdin = self.stdin.as_mut().expect("stdin still open");
        stdin.write_all(line.as_bytes()).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
    }

    fn send_json(&mut self, frame: Value) {
        self.send(&serde_json::to_string(&frame).unwrap());
    }

    /// The next stdout line raw (the wire bound is asserted on the line
    /// bytes before any parsing); panics past the deadline so a hung server
    /// fails the test instead of stalling the suite.
    fn next_line(&self, seconds: u64) -> String {
        match self.responses.recv_timeout(Duration::from_secs(seconds)) {
            Ok(line) => line,
            Err(RecvTimeoutError::Timeout) => panic!("no server response within {seconds}s"),
            Err(RecvTimeoutError::Disconnected) => {
                panic!("server stdout closed before the expected response");
            }
        }
    }

    fn handshake(&mut self) {
        self.send_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "bound-test", "version": "0"},
            },
        }));
        let init: Value =
            serde_json::from_str(&self.next_line(10)).expect("initialize answers with JSON");
        assert_eq!(
            init["result"]["protocolVersion"], PROTOCOL_VERSION,
            "the requested protocol revision is echoed"
        );
        self.send_json(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    }

    /// One `query` round-trip; stdin stays open across it (a request pending
    /// at EOF is dropped). The id may be any JSON-RPC id shape — number or
    /// string; the server echoes it in the reply envelope. Returns the raw
    /// reply LINE with its trailing newline.
    fn call_query(&mut self, id: Value, sql: &str) -> String {
        self.call_query_with_meta(id, None, sql)
    }

    /// The same, with `meta` riding the call params as `_meta` (None keeps
    /// the legacy wire shape): rmcp only keeps the `resultType` discriminator
    /// on the reply when the request itself declares a 2026-07-28+ protocol.
    fn call_query_with_meta(&mut self, id: Value, meta: Option<Value>, sql: &str) -> String {
        let mut params = json!({
            "name": "query",
            "arguments": {"profile": "local", "sql": sql},
        });
        if let Some(meta) = meta {
            params["_meta"] = meta;
        }
        self.send_json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": params,
        }));
        let line = self.next_line(120);
        let reply: Value = serde_json::from_str(&line).expect("the reply line is JSON");
        assert_eq!(reply["id"], id, "the reply answers this call");
        line
    }

    /// Close stdin and require exit 0 within 15 s (a parallel test run can
    /// starve this thread's wall clock, and the last frame was megabytes).
    fn close_and_expect_exit(&mut self, expected: i32) {
        self.stdin = None;
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = self.child.try_wait().expect("poll child") {
                assert_eq!(
                    status.code(),
                    Some(expected),
                    "server exit code after stdin EOF"
                );
                break;
            }
            assert!(
                Instant::now() < deadline,
                "server did not exit within 5s of stdin EOF"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
