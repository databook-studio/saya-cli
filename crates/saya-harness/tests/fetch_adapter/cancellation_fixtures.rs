use super::*;

/// The loop-assigned context, as `turn_tools` builds it: the lane's cap and
/// the turn's token.
pub(super) fn context(token: &CancellationToken) -> ToolExecutionContext {
    ToolExecutionContext {
        result_cap: FetchLimits::tool_lane_cap(),
        cancellation: token.clone(),
    }
}

/// A transport whose request/headers wait never resolves, signalling the
/// test when `get` is entered so the cancel lands while the fetch waits.
pub(super) struct HeldHeadersNet {
    pub(super) entered: Arc<Notify>,
    pub(super) calls: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl FetchTransport for HeldHeadersNet {
    async fn resolve(&self, _: &str) -> Result<Vec<IpAddr>, FetchTransportError> {
        Ok(vec![IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7))])
    }

    async fn get(&self, request: FetchRequest) -> Result<WireResponse, FetchTransportError> {
        self.calls
            .lock()
            .expect("calls")
            .push(request.url.as_str().to_owned());
        self.entered.notify_one();
        std::future::pending().await
    }
}

/// Headers arrive, one body chunk is served, and the next chunk never
/// resolves — the cancel must land mid-body-wait.
pub(super) struct HeldBodyNet {
    pub(super) first_chunk_served: Arc<Notify>,
    pub(super) calls: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl FetchTransport for HeldBodyNet {
    async fn resolve(&self, _: &str) -> Result<Vec<IpAddr>, FetchTransportError> {
        Ok(vec![IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7))])
    }

    async fn get(&self, request: FetchRequest) -> Result<WireResponse, FetchTransportError> {
        self.calls
            .lock()
            .expect("calls")
            .push(request.url.as_str().to_owned());
        Ok(WireResponse {
            status: 200,
            location: None,
            content_range: None,
            body: Box::new(HeldBody {
                first_chunk_served: Arc::clone(&self.first_chunk_served),
                served: false,
            }),
        })
    }
}

struct HeldBody {
    first_chunk_served: Arc<Notify>,
    served: bool,
}

#[async_trait]
impl FetchBody for HeldBody {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, FetchTransportError> {
        if self.served {
            std::future::pending().await
        } else {
            self.served = true;
            self.first_chunk_served.notify_one();
            Ok(Some(b"chunk".to_vec()))
        }
    }
}

/// Hop 1 answers 302 and cancels the turn's token before returning, as a
/// real cancel racing the redirect would: hop 2 must never be requested.
pub(super) struct RedirectCancelNet {
    pub(super) calls: Arc<Mutex<Vec<String>>>,
    pub(super) token: CancellationToken,
}

#[async_trait]
impl FetchTransport for RedirectCancelNet {
    async fn resolve(&self, _: &str) -> Result<Vec<IpAddr>, FetchTransportError> {
        Ok(vec![IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7))])
    }

    async fn get(&self, request: FetchRequest) -> Result<WireResponse, FetchTransportError> {
        let requested = request.url.as_str().to_owned();
        self.calls.lock().expect("calls").push(requested.clone());
        if requested == url("/doc") {
            self.token.cancel();
            Ok(WireResponse {
                status: 302,
                location: Some(url("/next")),
                content_range: None,
                body: Box::new(CannedBody {
                    chunks: VecDeque::new(),
                }),
            })
        } else {
            Ok(WireResponse {
                status: 200,
                location: None,
                content_range: None,
                body: Box::new(CannedBody {
                    chunks: VecDeque::from(vec![b"hop two".to_vec()]),
                }),
            })
        }
    }
}
