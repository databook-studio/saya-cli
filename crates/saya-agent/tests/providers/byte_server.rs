fn byte_server(chunks: Vec<Vec<u8>>) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_request(&mut stream);
        // The client under test is *expected* to hang up mid-body once the
        // stream cap trips, so writes here race the close: they either fail
        // with EPIPE or block on a socket buffer nobody is draining. The
        // timeout bounds the blocking case and the errors are ignored, because
        // a truncated write is the behaviour being exercised, not a fault.
        // Without both, `handle.join()` can wait forever — this test hung. The
        // timeout is well under the 5s the test itself asserts, so a blocked
        // write cannot push the run past its own deadline.
        let _ = stream.set_write_timeout(Some(Duration::from_millis(250)));
        let length: usize = chunks.iter().map(Vec::len).sum();
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
        );
        for chunk in chunks {
            if stream.write_all(&chunk).is_err() {
                break;
            }
            let _ = stream.flush();
        }
    });
    (base, handle)
}
