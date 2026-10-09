use super::{Reply, gemini, request, server};
use saya_agent::ChatProvider;

#[tokio::test]
async fn test_gemini_text_completion() {
    let (base, requests, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![r#"{"candidates":[{"content":{"parts":[{"text":"Hello"}]}}]}"#],
    }]);
    let response = gemini(base).complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(response.message.content, "Hello");
    let sent = &requests.lock().unwrap()[0];
    assert!(sent.contains("x-goog-api-key: key-sentinel"));
    assert!(sent.contains("systemInstruction"));
    assert!(sent.contains("functionDeclarations"));
    assert!(sent.contains("generationConfig"));
}

#[tokio::test]
async fn test_gemini_tool_call() {
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"schema_discovery","args":{"connection":"warehouse"}}}]}}]}"#,
        ],
    }]);
    let response = gemini(base).complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(response.message.tool_calls[0].name, "schema_discovery");
    assert_eq!(
        response.message.tool_calls[0].arguments["connection"],
        "warehouse"
    );
}
