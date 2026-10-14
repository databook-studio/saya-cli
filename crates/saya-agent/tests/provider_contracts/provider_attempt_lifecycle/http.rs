use super::support::{NoTools, Records, call_kinds};
use saya_agent::{
    AllowReadOnlyApproval, CancellationToken, EffectOutcome, EffectSettlement,
    OpenAiCompatibleProvider, ProviderSettings, run_agent_with_control,
};
use std::{sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const ANSWER: &str = concat!(
    "data: {\"choices\":[{\"delta\":{\"content\":\"answer\"},\"finish_reason\":null}]}\n\n",
    "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
    "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":2}}\n\n",
    "data: [DONE]\n\n"
);

pub(super) async fn local_server(
    records: Arc<Records>,
    statuses: Vec<u16>,
) -> (String, tokio::task::JoinHandle<Vec<usize>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let mut admissions_at_request = Vec::new();
        for status in statuses {
            let accepted =
                tokio::time::timeout(Duration::from_millis(250), listener.accept()).await;
            let Ok(Ok((mut socket, _))) = accepted else {
                break;
            };
            read_request(&mut socket).await;
            admissions_at_request.push(
                records
                    .attempts_seen
                    .load(std::sync::atomic::Ordering::SeqCst),
            );
            let body = if status == 200 { ANSWER } else { "" };
            let reason = match status {
                200 => "OK",
                429 => "Too Many Requests",
                _ => "Service Unavailable",
            };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nRetry-After: 0\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
        }
        admissions_at_request
    });
    (format!("http://{address}/v1"), task)
}

pub(super) async fn read_request(socket: &mut tokio::net::TcpStream) {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 2048];
    loop {
        let count = socket.read(&mut chunk).await.unwrap();
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.len() > 16 * 1024 {
            break;
        }
        if let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..header_end]).to_ascii_lowercase();
            let content_length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            if bytes.len() >= header_end + 4 + content_length {
                break;
            }
        }
    }
}

pub(super) fn provider(base_url: String, retry_delays: Vec<Duration>) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(
        ProviderSettings::new("test-model", Some(base_url)).with_retry_delays(retry_delays),
        Some("redacted-test-key"),
    )
    .unwrap()
}

#[tokio::test]
async fn each_actual_http_retry_is_admitted_before_the_local_server_observes_it() {
    let records = Arc::new(Records::default());
    let (base_url, server) = local_server(Arc::clone(&records), vec![503, 429, 200]).await;
    let provider = provider(base_url, vec![Duration::ZERO, Duration::ZERO]);
    let approval = AllowReadOnlyApproval;
    let output = run_agent_with_control(
        &provider,
        &NoTools,
        Records::request(),
        Vec::new(),
        records.strict_control(&approval, CancellationToken::new()),
    )
    .await
    .unwrap();
    let observed_admissions = server.await.unwrap();
    assert_eq!(output.answer, "answer");
    assert_eq!(observed_admissions, vec![1, 2, 3]);
    let physical = records.physical.lock().unwrap().clone();
    assert_eq!(physical.len(), 3);
    assert_eq!(
        call_kinds(&physical),
        vec![saya_agent::EffectCallKind::Answer; 3]
    );
    assert_eq!(
        physical
            .iter()
            .map(|(_, attempt)| attempt.ordinal())
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    let parent = records.models.lock().unwrap()[0];
    assert!(
        physical
            .iter()
            .all(|(_, attempt)| attempt.parent() == parent)
    );
    let model_settlement = records
        .settlements
        .lock()
        .unwrap()
        .iter()
        .find_map(|(_, settlement)| match settlement {
            EffectSettlement::ModelRequest { usage_complete, .. } => Some(*usage_complete),
            _ => None,
        })
        .unwrap();
    assert!(
        !model_settlement,
        "retry responses without usage make total partial"
    );
    let physical_outcomes = records
        .settlements
        .lock()
        .unwrap()
        .iter()
        .filter_map(|(_, settlement)| match settlement {
            EffectSettlement::PhysicalAttempt { outcome, .. } => Some(*outcome),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        physical_outcomes,
        vec![
            EffectOutcome::Unknown,
            EffectOutcome::Unknown,
            EffectOutcome::Succeeded
        ]
    );
}
