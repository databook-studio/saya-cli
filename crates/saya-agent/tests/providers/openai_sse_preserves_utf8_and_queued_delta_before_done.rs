#[tokio::test]
async fn openai_sse_preserves_utf8_and_queued_delta_before_done() {
    let body = b"data: {\"choices\":[{\"delta\":{\"content\":\"planet \xf0\x9f\x8c\x8d\"}}]}\n\ndata: [DONE]\n\n";
    let split = body.iter().position(|byte| *byte == 0xf0).unwrap() + 2;
    let (base, handle) = byte_server(vec![body[..split].to_vec(), body[split..].to_vec()]);
    assert_eq!(
        openai(base)
            .complete(request())
            .await
            .unwrap()
            .message
            .content,
        "planet 🌍"
    );
    handle.join().unwrap();
}
