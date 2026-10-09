use super::State;
use crate::{CancellationToken, ProviderError, ProviderEvent};
use futures_util::StreamExt;
use std::time::Duration;

pub(super) async fn next<S>(
    mut value: (S, State, CancellationToken, Duration),
) -> Option<(
    Result<ProviderEvent, ProviderError>,
    (S, State, CancellationToken, Duration),
)>
where
    S: futures_util::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Unpin,
{
    loop {
        if value.2.is_cancelled() {
            value.1.done = true;
            return Some((Err(ProviderError::Cancelled), value));
        }
        if let Some(event) = value.1.pending.pop_front() {
            return Some((Ok(event), value));
        }
        if value.1.deferred.is_some() && (value.1.done || !value.1.draining) {
            value.1.done = true;
            let error = value.1.deferred.take()?;
            return Some((Err(error), value));
        }
        if value.1.done {
            return None;
        }
        // Per-chunk idle budget instead of a total cap (see anthropic_stream).
        let item = tokio::select! {
            _ = value.2.cancelled() => return Some((Err(ProviderError::Cancelled), value)),
            item = tokio::time::timeout(value.3, value.0.next()) => match item {
                Ok(item) => item,
                Err(_) => {
                    value.1.done = true;
                    if value.1.draining {
                        continue;
                    }
                    return Some((
                        Err(ProviderError::Request("provider stream stalled".into())),
                        value,
                    ));
                }
            }
        };
        let Some(chunk) = item else {
            value.1.done = true;
            if value.1.draining {
                continue;
            }
            return Some((Err(ProviderError::InvalidResponse), value));
        };
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(_) => {
                value.1.done = true;
                if value.1.draining {
                    continue;
                }
                return Some((
                    Err(ProviderError::Request("network request failed".into())),
                    value,
                ));
            }
        };
        if let Err(error) = value.1.push(&chunk) {
            if value.1.draining {
                // A bad trailer never replaces the typed truncation.
                value.1.done = true;
                continue;
            }
            if value.1.pending.is_empty() {
                value.1.done = true;
                return Some((Err(error), value));
            }
            // Events queued earlier in this read (usage) are delivered first.
            value.1.deferred = Some(error);
        }
    }
}
