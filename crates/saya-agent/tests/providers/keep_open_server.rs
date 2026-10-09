fn keep_open_server(body: &'static str) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_request(&mut stream);
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: keep-alive\r\n\r\n{}", body.len() + 1, body).unwrap();
        stream.flush().unwrap();
        thread::sleep(Duration::from_millis(200));
    });
    (base, handle)
}
