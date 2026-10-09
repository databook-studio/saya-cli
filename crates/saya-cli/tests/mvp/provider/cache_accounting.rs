/// A provider that *reports* zero cached tokens — a cold cache — must show
/// that as a number on the stream, distinguishable from the sibling test's
/// absent report. A cache hit rate computed over the two has to tell "0%"
/// from "unknown"; the `Option` exists precisely so the two cannot collapse.
#[test]
fn ask_reports_a_zero_cache_hit_as_zero_not_null() {
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
        // Same shape as the sibling test, with the detail object present but
        // the cached count zero.
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"answer from mock\"}}]}\n\n\
                    data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\
                    \"prompt_tokens_details\":{\"cached_tokens\":0}}}\n\n\
                    data: [DONE]\n\n";
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
    });
    let root = std::env::temp_dir().join(format!("saya-cli-ask-zero-usage-{}", std::process::id()));
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
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let usage_line = stdout
        .lines()
        .find(|line| line.contains("\"event\":\"usage\""))
        .unwrap_or_else(|| panic!("no usage event on the ndjson stream: {stdout}"));
    assert!(
        usage_line.contains(r#""cached_input_tokens":0"#),
        "a reported zero must reach the stream as a number: {usage_line}"
    );
    assert!(
        !usage_line.contains(r#""cached_input_tokens":null"#),
        "a reported zero must not be serialised as null: {usage_line}"
    );
    assert!(output.stderr.is_empty());
    let _ = std::fs::remove_dir_all(root);
}

/// A provider that sends no usage chunk must emit no usage event — an absent
/// report is not a zero, and an honest "unknown" beats a fabricated 0% cache
/// hit rate on the stream.
#[test]
fn ask_emits_no_usage_event_when_the_provider_reports_none() {
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
        // Same shape as the sibling test, minus the trailing usage chunk.
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"answer from mock\"}}]}\n\ndata: [DONE]\n\n";
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
    });
    let root = std::env::temp_dir().join(format!("saya-cli-ask-no-usage-{}", std::process::id()));
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
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("answer from mock"), "{stdout}");
    assert!(
        !stdout.contains("\"event\":\"usage\""),
        "a provider that reports no usage must not produce a usage event: {stdout}"
    );
    assert!(output.stderr.is_empty());
    let _ = std::fs::remove_dir_all(root);
}
