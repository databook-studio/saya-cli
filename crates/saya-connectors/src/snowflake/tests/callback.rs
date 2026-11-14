use super::*;

#[tokio::test]
async fn callback_accepts_empty_preconnect_and_fragmented_form_post() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let sender = tokio::spawn(async move {
        let _ = TcpStream::connect(address).await.unwrap();
        sleep(Duration::from_millis(10)).await;
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream
            .write_all(b"POST /callback HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: 21\r\n\r\ntoken=")
            .await
            .unwrap();
        sleep(Duration::from_millis(10)).await;
        stream.write_all(b"a%2Bb%26c%3D%3F").await.unwrap();
    });
    let token = super::super::sso_callback::capture_token(&listener, Duration::from_secs(1))
        .await
        .unwrap();
    sender.await.unwrap();
    assert_eq!(token, "a+b&c=?");
}

#[tokio::test]
async fn callback_rejects_wrong_method_content_type_and_malformed_request() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let sender = tokio::spawn(async move {
        for request in [
            "PUT /callback HTTP/1.1\r\nHost: localhost\r\n\r\n",
            "POST /callback HTTP/1.1\r\nHost: localhost\r\nContent-Type: text/plain\r\nContent-Length: 8\r\n\r\ntoken=no",
            "BROKEN\r\n\r\n",
            "GET /callback?token=final%2Btoken HTTP/1.1\r\nHost: localhost\r\n\r\n",
        ] {
            let mut stream = TcpStream::connect(address).await.unwrap();
            stream.write_all(request.as_bytes()).await.unwrap();
            sleep(Duration::from_millis(5)).await;
        }
    });
    let token = super::super::sso_callback::capture_token(&listener, Duration::from_secs(1))
        .await
        .unwrap();
    sender.await.unwrap();
    assert_eq!(token, "final+token");
}

#[tokio::test]
async fn callback_rejects_duplicate_fields_oversized_input_and_times_out() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let sender = tokio::spawn(async move {
        for request in [
            "POST /callback HTTP/1.1\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: 8\r\nContent-Length: 8\r\n\r\ntoken=no",
            "POST /callback HTTP/1.1\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: 8\r\n\r\ntoken=no",
            "GET /callback?token=one&token=two HTTP/1.1\r\nHost: localhost\r\n\r\n",
            "POST /callback HTTP/1.1\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: 5\r\n\r\ntoken=extra",
            &format!(
                "GET /callback?token=large HTTP/1.1\r\nX-Large: {}\r\n\r\n",
                "x".repeat(17 * 1024)
            ),
            "POST /callback HTTP/1.1\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: 9000\r\n\r\n",
            "GET /callback?token=final HTTP/1.1\r\nHost: localhost\r\n\r\n",
        ] {
            let mut stream = TcpStream::connect(address).await.unwrap();
            stream.write_all(request.as_bytes()).await.unwrap();
            sleep(Duration::from_millis(5)).await;
        }
    });
    let token = super::super::sso_callback::capture_token(&listener, Duration::from_secs(1))
        .await
        .unwrap();
    sender.await.unwrap();
    assert_eq!(token, "final");

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    assert!(
        super::super::sso_callback::capture_token(&listener, Duration::from_millis(20))
            .await
            .is_err()
    );
}
