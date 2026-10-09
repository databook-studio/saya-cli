use super::*;

/// A browser sends a CORS preflight before posting the token cross-origin from
/// the Snowflake callback page. Answering it with a rejection makes the browser
/// abandon the post, so sign-in stalls with no token ever arriving. The
/// preflight is answered and the listener keeps waiting for the real request.
#[tokio::test]
async fn callback_answers_a_cors_preflight_and_keeps_waiting() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let sender = tokio::spawn(async move {
        let mut preflight = TcpStream::connect(address).await.unwrap();
        preflight
            .write_all(
                b"OPTIONS /callback HTTP/1.1\r\nHost: localhost\r\nOrigin: https://acct.snowflakecomputing.com\r\nAccess-Control-Request-Method: POST\r\n\r\n",
            )
            .await
            .unwrap();
        let mut answer = String::new();
        preflight.read_to_string(&mut answer).await.unwrap();
        sleep(Duration::from_millis(5)).await;

        let body = "token=after-preflight";
        let mut post = TcpStream::connect(address).await.unwrap();
        post.write_all(format!("POST /callback HTTP/1.1\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes())
            .await
            .unwrap();
        answer
    });

    let token = super::super::sso_callback::capture_token(&listener, Duration::from_secs(2))
        .await
        .unwrap();
    let answer = sender.await.unwrap();

    assert!(
        answer.starts_with("HTTP/1.1 200"),
        "a preflight must be answered, not rejected: {answer}"
    );
    assert!(
        answer
            .to_ascii_lowercase()
            .contains("access-control-allow-origin"),
        "the answer must carry the allow headers the browser is asking for: {answer}"
    );
    assert_eq!(token, "after-preflight");
}
