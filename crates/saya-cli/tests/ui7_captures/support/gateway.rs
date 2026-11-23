// A gateway that streams `ANSWER`, then reports usage: input tokens against the
// model's window, and a provider-reported cost. Nothing reaches a network.

fn provider(reports_cost: bool) -> (String, Sender<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("the mock provider binds");
    let address = format!("http://{}", listener.local_addr().expect("its address"));
    let (stop, stopped) = channel();
    std::thread::spawn(move || {
        listener.set_nonblocking(true).expect("nonblocking listener");
        loop {
            match listener.accept() {
                Ok((stream, _)) => answer_one(stream, reports_cost),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if stopped.try_recv().is_ok() {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return,
            }
        }
    });
    (format!("{address}/v1"), stop)
}

fn answer_one(mut stream: std::net::TcpStream, reports_cost: bool) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    if !drain_request(&mut stream) {
        return;
    }
    let mut body = String::new();
    for chunk in ANSWER.split_inclusive(' ') {
        let delta = serde_json::json!({"choices": [{"delta": {"content": chunk}}]});
        body.push_str(&format!("data: {delta}\n\n"));
        std::thread::sleep(Duration::from_millis(8));
    }
    // The cost is the gateway's to report or not: a provider that prices the
    // request says so, and one that does not leaves the session's spend unknown.
    let usage = match reports_cost {
        true => serde_json::json!({"choices": [], "usage": {
            "prompt_tokens": 51_200, "completion_tokens": 140, "cost": 0.0312}}),
        false => serde_json::json!({"choices": [], "usage": {
            "prompt_tokens": 51_200, "completion_tokens": 140}}),
    };
    body.push_str(&format!("data: {usage}\n\ndata: [DONE]\n\n"));
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\n\
         Connection: close\r\n\r\n",
        body.len()
    );
    let _ = write!(stream, "{head}{body}");
    let _ = stream.flush();
}

fn drain_request(stream: &mut std::net::TcpStream) -> bool {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        if let Some(head) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let length = String::from_utf8_lossy(&bytes[..head])
                .lines()
                .find_map(|line| match line.split_once(':') {
                    Some((name, value)) if name.eq_ignore_ascii_case("content-length") => {
                        value.trim().parse::<usize>().ok()
                    }
                    _ => None,
                })
                .unwrap_or(0);
            if bytes.len() >= head + 4 + length {
                return true;
            }
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return false,
            Ok(count) => bytes.extend_from_slice(&chunk[..count]),
        }
    }
}
