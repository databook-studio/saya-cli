use std::time::Duration;

use tokio::{net::TcpListener, time::timeout};

use super::{errors, sso_callback_request};
use saya_types::ConnectionError;

pub(crate) async fn capture_token(
    listener: &TcpListener,
    duration: Duration,
) -> Result<String, ConnectionError> {
    timeout(duration, async {
        loop {
            let (mut stream, _) = listener.accept().await.map_err(|_| errors::auth())?;
            let request = timeout(
                Duration::from_secs(2),
                sso_callback_request::read(&mut stream),
            )
            .await;
            let token = request.ok().and_then(Result::ok).flatten();
            if let Some(token) = token {
                let _ =
                    sso_callback_request::reply(&mut stream, 200, "Authentication complete").await;
                return Ok(token);
            }
            let _ =
                sso_callback_request::reply(&mut stream, 400, "Authentication request rejected")
                    .await;
        }
    })
    .await
    .map_err(|_| errors::auth())?
}
