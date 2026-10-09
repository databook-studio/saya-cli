struct TestServer {
    stdin: Option<ChildStdin>,
    responses: Receiver<String>,
    child: Child,
}

impl TestServer {
    /// Spawn the real binary serving MCP over stdio; `extra` are the
    /// subcommand arguments (e.g. `["mcp", "serve", "--profile", "local"]`).
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
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(line) => {
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

    /// The next stdout line as a JSON-RPC 2.0 value; panics past the deadline
    /// so a hung server fails the test instead of stalling the suite.
    fn next_json(&self, seconds: u64) -> Value {
        let line = self.next_line(seconds);
        let value: Value = serde_json::from_str(&line)
            .unwrap_or_else(|error| panic!("stdout line is not JSON: {error}: {line}"));
        assert_eq!(
            value["jsonrpc"], "2.0",
            "every stdout line is JSON-RPC 2.0: {line}"
        );
        value
    }

    fn next_line(&self, seconds: u64) -> String {
        match self.responses.recv_timeout(Duration::from_secs(seconds)) {
            Ok(line) => line,
            Err(RecvTimeoutError::Timeout) => {
                panic!("no server response within {seconds}s");
            }
            Err(RecvTimeoutError::Disconnected) => {
                panic!("server stdout closed before the expected response");
            }
        }
    }

    /// Whether a response for `id` arrived within the window; scans past any
    /// other frames (their ids are reported back for diagnostics).
    fn got_response_for(&self, id: i64, milliseconds: u64) -> bool {
        let deadline = Instant::now() + Duration::from_millis(milliseconds);
        while Instant::now() < deadline {
            match self.responses.recv_timeout(Duration::from_millis(
                (deadline - Instant::now()).as_millis() as u64,
            )) {
                Ok(line) => {
                    if let Ok(value) = serde_json::from_str::<Value>(&line)
                        && value["id"].as_i64() == Some(id)
                    {
                        return true;
                    }
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        false
    }

    /// Assert no response for `id` arrives within the window.
    fn expect_no_response_for(&self, id: i64, milliseconds: u64) {
        assert!(
            !self.got_response_for(id, milliseconds),
            "the cancelled call must never be answered (id {id})"
        );
    }

    /// Close stdin and require exit 0 within 5 s (invariant 4).
    fn close_and_expect_exit(&mut self, expected: i32) {
        self.stdin = None;
        let deadline = Instant::now() + Duration::from_secs(5);
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
        // Nothing but consumed responses may sit on stdout.
        if let Ok(line) = self.responses.recv_timeout(Duration::from_millis(200)) {
            panic!("unexpected stdout line after the last response: {line}");
        }
    }

    /// Wait for the child to exit on its own (used for refused startups). The
    /// deadline is generous — this pins the exit code, not the latency, and a
    /// parallel test run can starve this thread's wall clock.
    fn wait_for_exit(&mut self) -> i32 {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = self.child.try_wait().expect("poll child") {
                return status.code().expect("exit code");
            }
            assert!(Instant::now() < deadline, "server did not exit within 15s");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn stderr(&mut self) -> String {
        let mut stderr = self.child.stderr.take().expect("piped stderr");
        let mut buffer = String::new();
        std::io::Read::read_to_string(&mut stderr, &mut buffer).unwrap();
        buffer
    }

    fn handshake(&mut self) {
        self.send(&initialize_request());
        let init = self.next_json(10);
        assert_eq!(
            init["result"]["protocolVersion"], PROTOCOL_VERSION,
            "the requested protocol revision is echoed"
        );
        assert_eq!(
            init["result"]["serverInfo"]["name"],
            "saya",
            "initialize names the server: {}",
            serde_json::to_string(&init).unwrap()
        );
        assert_eq!(
            init["result"]["serverInfo"]["version"],
            env!("CARGO_PKG_VERSION"),
            "initialize reports the crate version"
        );
        assert!(
            init["result"]["capabilities"]["tools"].is_object(),
            "initialize advertises the tools capability: {}",
            serde_json::to_string(&init).unwrap()
        );
        self.send(initialized_notification());
    }

    /// `tools/list` tool names, order-independent.
    fn listed_tools(&mut self, id: i64) -> Vec<String> {
        self.send(&format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/list"}}"#
        ));
        let tools = self.next_json(10);
        assert_eq!(tools["id"], id);
        tools["result"]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name").to_string())
            .collect()
    }

    /// One tools/call round-trip; asserts the frame is JSON-RPC and returns it.
    fn call(&mut self, id: i64, name: &str, arguments: &str) -> Value {
        self.send(&format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{name}","arguments":{arguments}}}}}"#
        ));
        let call = self.next_json(30);
        assert_eq!(call["id"], id);
        call
    }

    /// One tools/call expected to answer as a tool-level error; returns the
    /// flattened text the client would show.
    fn call_error_text(&mut self, id: i64, name: &str, arguments: &str) -> String {
        let call = self.call(id, name, arguments);
        assert_eq!(
            call["result"]["isError"],
            true,
            "expected isError: {}",
            serde_json::to_string(&call).unwrap()
        );
        call["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }
}
