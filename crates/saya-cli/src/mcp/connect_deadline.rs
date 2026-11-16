//! Bounded connection setup shared by the data tools: a connect that has not
//! completed when the call is cancelled or the request deadline passes is
//! dropped, so a slow endpoint cannot hold the call — and its in-flight slot —
//! for the connector's own timeout. Nothing is executed against the database.

use std::future::Future;

use saya_connectors::DatabaseConnector;
use saya_types::ConnectionError;

/// Why connection setup stopped before it completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConnectionStop {
    Cancelled,
    TimedOut,
}

impl ConnectionStop {
    /// The pre-execution result text every data tool returns for this stop.
    pub(super) const fn message(self) -> &'static str {
        match self {
            Self::Cancelled => "the call was cancelled",
            Self::TimedOut => {
                "the database operation exceeded its 30-second cancellation-request deadline"
            }
        }
    }
}

pub(super) enum ConnectionSetup {
    Connected,
    Stopped(ConnectionStop),
    Failed(ConnectionError),
}

/// Races connection setup against the client's cancellation and the request
/// deadline. Cancellation and the deadline win over a connect that is ready in
/// the same poll; the losing connect future is dropped.
pub(super) async fn connect_bounded<C>(
    connector: &dyn DatabaseConnector,
    source_cancelled: C,
    deadline: tokio::time::Instant,
) -> ConnectionSetup
where
    C: Future<Output = ()>,
{
    tokio::select! {
        biased;
        _ = source_cancelled => ConnectionSetup::Stopped(ConnectionStop::Cancelled),
        _ = tokio::time::sleep_until(deadline) => {
            ConnectionSetup::Stopped(ConnectionStop::TimedOut)
        }
        result = connector.connect() => match result {
            Ok(()) => ConnectionSetup::Connected,
            Err(error) => ConnectionSetup::Failed(error),
        },
    }
}
