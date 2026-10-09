#[cfg(unix)]
pub(crate) fn drain_provider_request(stream: &mut std::net::TcpStream) -> String {
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
pub(crate) fn accept_provider(listener: &std::net::TcpListener) -> Option<std::net::TcpStream> {
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
pub(crate) fn wait_for_stream_timeout(
    buffer: &std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    needle: &[u8],
    label: &str,
    timeout: std::time::Duration,
) {
    use std::{thread, time::Instant};

    let deadline = Instant::now() + timeout;
    let needle_owned = needle.to_vec();
    loop {
        if buffer
            .lock()
            .unwrap()
            .windows(needle_owned.len())
            .any(|window| window == needle_owned.as_slice())
        {
            return;
        }
        if Instant::now() >= deadline {
            let full = {
                let locked = buffer.lock().unwrap();
                String::from_utf8_lossy(&locked).into_owned()
            };
            let start = full.len().saturating_sub(2000);
            panic!(
                "did not observe {label}; buffer ({}/full): {:?}",
                full.len(),
                &full[start..]
            );
        }
        thread::sleep(std::time::Duration::from_millis(10));
    }
}
