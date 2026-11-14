/// Serves one streaming request and writes `frames` `repeats` times, pausing
/// `interval` between passes. The pause is what distinguishes a stream that
/// keeps its transport alive (keep-alives, empty deltas, usage-only frames)
/// from one that has gone silent: the client sees chunks, not a gap.
fn paced_server(
    frames: Vec<&'static str>,
    interval: Duration,
    repeats: usize,
) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let body_len: usize = frames.iter().map(|frame| frame.len()).sum::<usize>() * repeats;
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_request(&mut stream);
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {body_len}\r\nContent-Type: text/event-stream\r\nConnection: keep-alive\r\n\r\n"
        );
        for _ in 0..repeats {
            for frame in &frames {
                if stream.write_all(frame.as_bytes()).is_err() {
                    return;
                }
            }
            if stream.flush().is_err() {
                return;
            }
            thread::sleep(interval);
        }
    });
    (base, handle)
}
