#[cfg(unix)]
pub(crate) fn wait_for_stream(
    buffer: &std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    needle: &[u8],
    label: &str,
) {
    wait_for_stream_timeout(buffer, needle, label, std::time::Duration::from_secs(5));
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
