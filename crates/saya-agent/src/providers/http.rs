use crate::{CancellationToken, ProviderError};
use reqwest::{RequestBuilder, Response, StatusCode};
use std::time::Duration;

pub(super) async fn send_stream(
    mut build: impl FnMut() -> RequestBuilder,
    delays: &[Duration],
    cancellation: &CancellationToken,
) -> Result<Response, ProviderError> {
    for attempt in 0..=delays.len() {
        let response = tokio::select! {
            _ = cancellation.cancelled() => return Err(ProviderError::Cancelled),
            response = build().send() => response,
        };
        match response {
            Ok(response) if response.status().is_success() => return Ok(response),
            Ok(response) if retryable(response.status()) && attempt < delays.len() => {
                wait(delays[attempt], cancellation).await?;
            }
            Ok(response) => {
                return Err(ProviderError::Request(format!(
                    "HTTP {}",
                    response.status().as_u16()
                )));
            }
            Err(_) if attempt < delays.len() => wait(delays[attempt], cancellation).await?,
            Err(_) => return Err(ProviderError::Request("network request failed".into())),
        }
    }
    Err(ProviderError::Request("network request failed".into()))
}

async fn wait(delay: Duration, cancellation: &CancellationToken) -> Result<(), ProviderError> {
    tokio::select! {
        _ = cancellation.cancelled() => Err(ProviderError::Cancelled),
        _ = tokio::time::sleep(delay) => Ok(()),
    }
}

fn retryable(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}
