//! Concurrent scripted root completion with an intentionally unanswered child request.
use serde_json::json;
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Notify, oneshot},
    task::JoinHandle,
};

pub(super) struct Provider {
    pub(super) endpoint: String,
    pub(super) child_entered: Arc<Notify>,
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}
impl Provider {
    pub(super) async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let child_entered = Arc::new(Notify::new());
        let entered = Arc::clone(&child_entered);
        let (stop, mut stopped) = oneshot::channel();
        let task = tokio::spawn(async move {
            let mut root_turn = 0;
            let mut held = Vec::new();
            for _ in 0..3 {
                let (mut stream, _) = tokio::select! { _=&mut stopped => return, accepted=listener.accept()=>accepted.unwrap() };
                let body = read_request(&mut stream).await;
                if body.contains("UNTRUSTED CHILD TASK") {
                    held.push(stream);
                    entered.notify_one();
                    continue;
                }
                let response = if root_turn == 0 {
                    json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call-task","type":"function","function":{"name":"helpers","arguments":json!({"description":"inspect assigned work","prompt":"write for held-child","subagent_type":"general","agent_id":"held-child"}).to_string()}}]},"finish_reason":"tool_calls"}]})
                } else {
                    json!({"choices":[{"index":0,"delta":{"content":"root finished without explicitly joining"},"finish_reason":"stop"}]})
                };
                root_turn += 1;
                let response = format!(
                    "data: {response}\n\ndata: {}\n\ndata: [DONE]\n\n",
                    json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5}})
                );
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                    response.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
            let _ = stopped.await;
        });
        Self {
            endpoint,
            child_entered,
            stop: Some(stop),
            task,
        }
    }
}
impl Drop for Provider {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        self.task.abort();
    }
}
async fn read_request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let count =
            tokio::time::timeout(std::time::Duration::from_secs(3), stream.read(&mut buffer))
                .await
                .unwrap()
                .unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&buffer[..count]);
        assert!(bytes.len() <= 256 * 1024);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let length = String::from_utf8_lossy(&bytes[..end])
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            if bytes.len() >= end + 4 + length {
                return String::from_utf8(bytes[end + 4..end + 4 + length].to_vec()).unwrap();
            }
        }
    }
}
