#[tokio::test]
async fn terminal_markers_finish_without_waiting_for_socket_close() {
    let (base, handle) = keep_open_server(
        "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n",
    );
    let response =
        tokio::time::timeout(Duration::from_millis(100), openai(base).complete(request()))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(response.message.content, "ok");
    handle.join().unwrap();
    let (base, handle) = keep_open_server("{\"message\":{\"content\":\"ok\"},\"done\":true}\n");
    let provider = OllamaProvider::new(ProviderSettings::new("test", Some(base))).unwrap();
    let response = tokio::time::timeout(Duration::from_millis(100), provider.complete(request()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.message.content, "ok");
    handle.join().unwrap();
}
