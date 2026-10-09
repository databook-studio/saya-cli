#[test]
fn ask_calls_configured_openai_compatible_provider() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 8192];
        let _ = stream.read(&mut request);
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"answer from mock\"}}]}\n\ndata: [DONE]\n\n";
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
    });
    let root = std::env::temp_dir().join(format!("saya-cli-ask-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let provider_state = root.join("provider-only-state.sqlite3");
    let database = root.join("ask.duckdb");
    duckdb::Connection::open(&database)
        .unwrap()
        .execute_batch("CREATE TABLE revenue (amount INTEGER); INSERT INTO revenue VALUES (7);")
        .unwrap();
    let connections = root.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'duckdb'\npath = '{}'\nread_only = true\n",
            database.display()
        ),
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "--non-interactive",
            "--format",
            "json",
            "--connections",
            connections.to_str().unwrap(),
            "--profile",
            "local",
            "ask",
            "show revenue",
        ])
        .env("SAYA_CONFIG_HOME", &root)
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"))
        .env("SAYA_API_KEY", "mock-secret")
        .env("SAYA_STATE_DB", &provider_state)
        .output()
        .unwrap();
    handle.join().unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("answer from mock"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("mock-secret"));
    assert!(output.stderr.is_empty());
    assert!(
        !provider_state.exists(),
        "provider-only ask must not create an agent-query audit"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// The OpenAI-compatible gateway sends `stream_options.include_usage` a
/// trailing usage-only chunk. `ask --format ndjson` must carry that report on
/// the stream, because a benchmark reading NDJSON cannot otherwise establish
/// whether prompt caching worked — the capability exists (`/usage` renders it
/// in a session) and simply never reached this surface.
///
/// `cached_input_tokens: 8` vs `input_tokens: 10` is also the distinction the
/// wire has to keep: a reported cache read is a *number*, and a provider that
/// reports none serializes `null`, so a 0% hit rate is never invented for a
/// run that never got an answer.
#[test]
fn ask_reports_usage_on_the_ndjson_stream() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 8192];
        let _ = stream.read(&mut request);
        // The answer delta, then the trailing usage-only chunk the gateway
        // appends when `include_usage` was asked for.
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"answer from mock\"}}]}\n\n\
                    data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\
                    \"prompt_tokens_details\":{\"cached_tokens\":8}}}\n\n\
                    data: [DONE]\n\n";
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
    });
    let root = std::env::temp_dir().join(format!("saya-cli-ask-usage-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let provider_state = root.join("provider-only-state.sqlite3");
    let database = root.join("ask.duckdb");
    duckdb::Connection::open(&database)
        .unwrap()
        .execute_batch("CREATE TABLE revenue (amount INTEGER); INSERT INTO revenue VALUES (7);")
        .unwrap();
    let connections = root.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'duckdb'\npath = '{}'\nread_only = true\n",
            database.display()
        ),
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "--non-interactive",
            "--format",
            "ndjson",
            "--connections",
            connections.to_str().unwrap(),
            "--profile",
            "local",
            "ask",
            "show revenue",
        ])
        .env("SAYA_CONFIG_HOME", &root)
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"))
        .env("SAYA_API_KEY", "mock-secret")
        .env("SAYA_STATE_DB", &provider_state)
        .output()
        .unwrap();
    handle.join().unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("answer from mock"), "{stdout}");
    let usage_line = stdout
        .lines()
        .find(|line| line.contains("\"event\":\"usage\""))
        .unwrap_or_else(|| panic!("no usage event on the ndjson stream: {stdout}"));
    assert!(usage_line.contains(r#""call":"answer""#), "{usage_line}");
    assert!(usage_line.contains(r#""input_tokens":10"#), "{usage_line}");
    assert!(usage_line.contains(r#""output_tokens":5"#), "{usage_line}");
    // The reported cache read must survive as a number, not as null or an
    // invented zero — the cache hit rate depends on it.
    assert!(
        usage_line.contains(r#""cached_input_tokens":8"#),
        "{usage_line}"
    );
    // Nothing else in this provider's report was cache creation or reasoning.
    assert!(
        usage_line.contains(r#""cache_creation_input_tokens":null"#),
        "{usage_line}"
    );
    assert!(
        usage_line.contains(r#""reasoning_tokens":null"#),
        "{usage_line}"
    );
    assert!(output.stderr.is_empty());
    let _ = std::fs::remove_dir_all(root);
}
