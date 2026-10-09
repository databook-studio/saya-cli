/// A loopback Ollama endpoint that answers a cross-host 307 must not have
/// its POST replayed to the redirect target: the default reqwest policy
/// follows up to 10 redirects and replays the body on 307, so an answering
/// redirect could pull the prompt — including database-derived context —
/// off the local classification onto another host without consent.
#[tokio::test]
async fn ollama_does_not_replay_the_post_to_a_cross_host_redirect() {
    use saya_agent::OllamaProvider;
    // Target host: captures any request body that reaches it, then answers a
    // terminal Ollama record.
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    target.set_nonblocking(true).unwrap();
    let target_base = format!("http://{}", target.local_addr().unwrap());
    let reached = Arc::new(Mutex::new(Vec::new()));
    let reached_copy = reached.clone();
    let target_handle = thread::spawn(move || {
        // Nonblocking accept with a bounded deadline: when the client
        // refuses the redirect (the secure behaviour) nothing ever
        // connects, and the capture must end rather than hang the suite.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            match target.accept() {
                Ok((mut stream, _)) => {
                    target.set_nonblocking(false).ok();
                    reached_copy.lock().unwrap().push(read_request(&mut stream));
                    let body = "{\"message\":{\"content\":\"redirected\"},\"done\":true}\n";
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .unwrap();
                    return;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(20));
                }
                Err(_) => return,
            }
        }
    });
    // Origin host: answers 307 to the target's chat URL.
    let origin = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin_base = format!("http://{}", origin.local_addr().unwrap());
    let origin_handle = thread::spawn(move || {
        let (mut stream, _) = origin.accept().unwrap();
        let _ = read_request(&mut stream);
        let location = format!("{target_base}/api/chat");
        let head = format!(
            "HTTP/1.1 307 Temporary Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(head.as_bytes()).unwrap();
        stream.flush().unwrap();
    });
    let provider = OllamaProvider::new(
        ProviderSettings::new("test", Some(origin_base)).with_retry_delays(vec![Duration::ZERO]),
    )
    .unwrap();
    let _ = provider.complete(request()).await;
    origin_handle.join().unwrap();
    target_handle.join().unwrap();
    let hits = reached.lock().unwrap().len();
    assert_eq!(
        hits, 0,
        "a cross-host 307 must not replay the Ollama POST to the redirect target"
    );
}
