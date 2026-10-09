fn redirect_capture(target_url: &str, terminal: &'static str) -> RedirectCapture {
    // Target host: captures any request that reaches it, then answers the
    // provider's terminal record.
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    target.set_nonblocking(true).unwrap();
    let target_base = format!("http://{}", target.local_addr().unwrap());
    let redirect_to = format!("{target_base}{target_url}");
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
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{terminal}",
                        terminal.len()
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
    // Origin host: answers 307 to the target's URL.
    let origin = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin_base = format!("http://{}", origin.local_addr().unwrap());
    let origin_handle = thread::spawn(move || {
        let (mut stream, _) = origin.accept().unwrap();
        let _ = read_request(&mut stream);
        let head = format!(
            "HTTP/1.1 307 Temporary Redirect\r\nLocation: {redirect_to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(head.as_bytes()).unwrap();
        stream.flush().unwrap();
    });
    RedirectCapture {
        origin_base,
        reached,
        origin_handle,
        target_handle,
    }
}
