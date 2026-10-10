use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

pub(super) struct LocalOpenAi {
    pub endpoint: String,
    requests: mpsc::UnboundedReceiver<usize>,
    responses: mpsc::UnboundedSender<Vec<u8>>,
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
    pub count: Arc<AtomicUsize>,
}

impl LocalOpenAi {
    pub async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, requests) = mpsc::unbounded_channel();
        let (responses, mut response_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let (stop, mut stop_rx) = oneshot::channel();
        let count = Arc::new(AtomicUsize::new(0));
        let task_count = Arc::clone(&count);
        let task = tokio::spawn(async move {
            loop {
                let accepted = tokio::select! {
                    _ = &mut stop_rx => break,
                    accepted = listener.accept() => accepted,
                };
                let Ok((mut stream, _)) = accepted else { break };
                let request = tokio::select! {
                    _ = &mut stop_rx => break,
                    request = read_request(&mut stream) => request,
                };
                if request.is_err() {
                    break;
                }
                let index = task_count.fetch_add(1, Ordering::SeqCst) + 1;
                if request_tx.send(index).is_err() {
                    break;
                }
                let response = tokio::select! {
                    _ = &mut stop_rx => break,
                    response = response_rx.recv() => response,
                };
                let Some(response) = response else { break };
                let _ = stream.write_all(&response).await;
                let _ = stream.shutdown().await;
            }
        });
        Self {
            endpoint: format!("http://{address}/v1"),
            requests,
            responses,
            stop: Some(stop),
            task,
            count,
        }
    }

    /// Bounded request wait so a stalled child surfaces as an assertion failure
    /// instead of an indefinite hang.
    pub async fn request(&mut self) -> Option<usize> {
        self.request_within(Duration::from_secs(5)).await
    }

    pub async fn request_within(&mut self, duration: Duration) -> Option<usize> {
        tokio::time::timeout(duration, self.requests.recv())
            .await
            .ok()
            .flatten()
    }

    pub fn respond_sse(&self, body: &str) {
        self.responses
            .send(response(200, "text/event-stream", body.as_bytes()))
            .unwrap();
    }

    pub fn respond_status(&self, status: u16) {
        self.responses
            .send(response(status, "text/plain", b"provider unavailable"))
            .unwrap();
    }

    pub async fn shutdown(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let _ = tokio::time::timeout(Duration::from_secs(1), &mut self.task).await;
    }
}

impl Drop for LocalOpenAi {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

async fn read_request(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    const MAX_REQUEST: usize = 1024 * 1024;
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "request ended",
            ));
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.len() > MAX_REQUEST {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request too large",
            ));
        }
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers =
        std::str::from_utf8(&bytes[..header_end]).map_err(|_| io::ErrorKind::InvalidData)?;
    let body_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    let total = header_end
        .checked_add(body_length)
        .filter(|total| *total <= MAX_REQUEST)
        .ok_or(io::ErrorKind::InvalidData)?;
    while bytes.len() < total {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "request body ended",
            ));
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    Ok(bytes)
}

fn response(status: u16, content_type: &str, body: &[u8]) -> Vec<u8> {
    let reason = if status == 200 {
        "OK"
    } else {
        "Internal Server Error"
    };
    let mut bytes = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len(),
    ).into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

pub(super) const TOOL_RESPONSE: &str = concat!(
    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[",
    "{\"index\":0,\"id\":\"call-0\",\"function\":{\"name\":\"inspect\",\"arguments\":\"{}\"}}",
    "],\"finish_reason\":\"tool_calls\"}]}\n\n",
    "data: [DONE]\n\n",
);

pub(super) const TWO_TOOL_RESPONSE: &str = concat!(
    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[",
    "{\"index\":0,\"id\":\"call-0\",\"function\":{\"name\":\"inspect\",\"arguments\":\"{}\"}},",
    "{\"index\":1,\"id\":\"call-1\",\"function\":{\"name\":\"inspect\",\"arguments\":\"{}\"}}",
    "],\"finish_reason\":\"tool_calls\"}]}\n\n",
    "data: [DONE]\n\n",
);

pub(super) const ANSWER_RESPONSE: &str = concat!(
    "data: {\"choices\":[{\"delta\":{\"content\":\"completed\"},\"finish_reason\":\"stop\"}]}\n\n",
    "data: [DONE]\n\n",
);
