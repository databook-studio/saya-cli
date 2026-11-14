use saya_cli::{GlobalOptions, approval_name};

#[cfg(unix)]
struct InteractiveChildGuard(Option<std::process::Child>);

#[cfg(unix)]
impl Drop for InteractiveChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            if child.try_wait().unwrap().is_none() {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}

#[cfg(unix)]
fn drain_provider_request(stream: &mut std::net::TcpStream) -> String {
    use std::io::Read;

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
                return String::from_utf8_lossy(&bytes[head + 4..head + 4 + length]).into_owned();
            }
        }
        let count = match stream.read(&mut chunk) {
            Ok(count) => count,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                return String::from_utf8_lossy(&bytes).into_owned();
            }
            Err(error) => panic!("provider request reads: {error}"),
        };
        if count == 0 {
            return String::new();
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
}

#[cfg(unix)]
fn accept_provider(listener: &std::net::TcpListener) -> Option<std::net::TcpStream> {
    use std::{
        io, thread,
        time::{Duration, Instant},
    };

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).ok()?;
                stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
                stream
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .ok()?;
                return Some(stream);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return None;
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return None,
        }
    }
}

#[cfg(unix)]
fn wait_for_stream(buffer: &std::sync::Arc<std::sync::Mutex<Vec<u8>>>, needle: &[u8], label: &str) {
    use std::{
        thread,
        time::{Duration, Instant},
    };

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if buffer
            .lock()
            .unwrap()
            .windows(needle.len())
            .any(|window| window == needle)
        {
            return;
        }
        assert!(Instant::now() < deadline, "did not observe {label}");
        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(unix)]
fn tool_collection_limit_frame() -> String {
    let calls = (0..=256)
        .map(|index| {
            format!(
                "{{\"index\":{index},\"id\":\"limit-{index}\",\"function\":{{\"name\":\"bounded_sql_query\",\"arguments\":\"{{}}\"}}}}"
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("data: {{\"choices\":[{{\"delta\":{{\"tool_calls\":[{calls}]}}}}]}}\n\n")
}

include!("session/startup_policy.rs");
include!("session/sigint_cancellation.rs");
include!("session/terminal_provider_error.rs");
include!("session/usage_zero.rs");
include!("session/failed_attempt_usage.rs");
include!("session/learning_receipt.rs");
