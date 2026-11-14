use std::time::Duration;

use saya_agent::{
    CancellationToken, ChatProvider, GeminiProvider, ProviderError, ProviderSettings,
};

use super::request;

#[tokio::test]
async fn gemini_establishment_timeout_is_not_a_client_total_timeout() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;
        drop(socket);
    });
    let provider = GeminiProvider::new(
        ProviderSettings::new("gemini-x", Some(format!("{base}/v1beta")))
            .with_retry_delays(Vec::new())
            .with_timeout(Duration::from_millis(20)),
        Some("key-sentinel"),
    )
    .unwrap();

    let error = provider.complete(request()).await.unwrap_err();
    assert!(
        matches!(&error, ProviderError::Request(message) if message.contains("timed out")),
        "unexpected establishment error: {error:?}"
    );
    server.await.unwrap();
}

#[tokio::test]
async fn gemini_stream_cancellation_interrupts_establishment() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;
        drop(socket);
    });
    let provider = GeminiProvider::new(
        ProviderSettings::new("gemini-x", Some(format!("{base}/v1beta")))
            .with_retry_delays(Vec::new())
            .with_timeout(Duration::from_secs(5)),
        Some("key-sentinel"),
    )
    .unwrap();
    let cancellation = CancellationToken::new();
    let task_cancellation = cancellation.clone();
    let task = tokio::spawn(async move { provider.stream(request(), task_cancellation).await });
    tokio::time::sleep(Duration::from_millis(20)).await;
    cancellation.cancel();

    let result = tokio::time::timeout(Duration::from_millis(100), task)
        .await
        .expect("cancellation must finish promptly")
        .unwrap();
    let error = match result {
        Ok(_) => panic!("cancellation must interrupt the Gemini request"),
        Err(error) => error,
    };
    assert_eq!(error, ProviderError::Cancelled);
    server.await.unwrap();
}
