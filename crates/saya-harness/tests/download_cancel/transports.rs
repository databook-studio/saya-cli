use super::*;

/// The request/headers wait never resolves; the test cancels while the
/// download waits inside `get`.
pub(super) struct HeldGetNet {
    pub(super) entered: Arc<Notify>,
    pub(super) calls: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl FetchTransport for HeldGetNet {
    async fn resolve(&self, _: &str) -> Result<Vec<IpAddr>, FetchTransportError> {
        Ok(vec![RESOLVED])
    }

    async fn get(&self, request: FetchRequest) -> Result<WireResponse, FetchTransportError> {
        logged(request.url.as_str(), &self.calls);
        self.entered.notify_one();
        std::future::pending().await
    }
}

/// Serves the first `stall_after` chunks of the content, then holds forever —
/// signalling when the hold begins, so the cancel lands mid-body, after the
/// served chunks were fully written and claimed.
pub(super) struct StalledNet {
    pub(super) content: Arc<Vec<u8>>,
    pub(super) chunk: usize,
    pub(super) stall_after: usize,
    pub(super) held: Arc<Notify>,
    pub(super) calls: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl FetchTransport for StalledNet {
    async fn resolve(&self, _: &str) -> Result<Vec<IpAddr>, FetchTransportError> {
        Ok(vec![RESOLVED])
    }

    async fn get(&self, request: FetchRequest) -> Result<WireResponse, FetchTransportError> {
        logged(request.url.as_str(), &self.calls);
        let chunks: VecDeque<Vec<u8>> = self
            .content
            .chunks(self.chunk)
            .take(self.stall_after)
            .map(<[u8]>::to_vec)
            .collect();
        Ok(WireResponse {
            status: 200,
            location: None,
            content_range: None,
            body: Box::new(StalledBody {
                chunks,
                held: Arc::clone(&self.held),
            }),
        })
    }
}

struct StalledBody {
    chunks: VecDeque<Vec<u8>>,
    held: Arc<Notify>,
}

#[async_trait]
impl FetchBody for StalledBody {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, FetchTransportError> {
        match self.chunks.pop_front() {
            Some(chunk) => Ok(Some(chunk)),
            None => {
                self.held.notify_one();
                std::future::pending().await
            }
        }
    }
}

/// Serves every chunk; the end-of-stream call cancels the turn's token
/// before returning, as a real cancel racing the end of the body would.
pub(super) struct EndCancelNet {
    pub(super) content: Arc<Vec<u8>>,
    pub(super) chunk: usize,
    pub(super) cancellation: CancellationToken,
    pub(super) calls: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl FetchTransport for EndCancelNet {
    async fn resolve(&self, _: &str) -> Result<Vec<IpAddr>, FetchTransportError> {
        Ok(vec![RESOLVED])
    }

    async fn get(&self, request: FetchRequest) -> Result<WireResponse, FetchTransportError> {
        logged(request.url.as_str(), &self.calls);
        Ok(WireResponse {
            status: 200,
            location: None,
            content_range: None,
            body: Box::new(EndCancelBody {
                chunks: self
                    .content
                    .chunks(self.chunk)
                    .map(<[u8]>::to_vec)
                    .collect(),
                cancellation: self.cancellation.clone(),
            }),
        })
    }
}

struct EndCancelBody {
    chunks: VecDeque<Vec<u8>>,
    cancellation: CancellationToken,
}

#[async_trait]
impl FetchBody for EndCancelBody {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, FetchTransportError> {
        match self.chunks.pop_front() {
            Some(chunk) => Ok(Some(chunk)),
            None => {
                self.cancellation.cancel();
                Ok(None)
            }
        }
    }
}

/// The cooperative resume fake: a ranged GET is answered with a 206 whose
/// `Content-Range` names the asked offset and a body of the slice from it;
/// a full GET answers 200 with the whole content. Logs the ranges asked.
pub(super) struct RangedNet {
    pub(super) content: Arc<Vec<u8>>,
    pub(super) chunk: usize,
    pub(super) calls: Arc<Mutex<Vec<String>>>,
    pub(super) ranges: Arc<Mutex<Vec<Option<u64>>>>,
}

#[async_trait]
impl FetchTransport for RangedNet {
    async fn resolve(&self, _: &str) -> Result<Vec<IpAddr>, FetchTransportError> {
        Ok(vec![RESOLVED])
    }

    async fn get(&self, request: FetchRequest) -> Result<WireResponse, FetchTransportError> {
        logged(request.url.as_str(), &self.calls);
        self.ranges
            .lock()
            .expect("ranges")
            .push(request.range_start);
        let (status, content_range, served): (u16, Option<String>, &[u8]) =
            match request.range_start {
                Some(start) => {
                    let start = start as usize;
                    (
                        206,
                        Some(format!(
                            "bytes {start}-{}/{}",
                            self.content.len() - 1,
                            self.content.len()
                        )),
                        &self.content[start..],
                    )
                }
                None => (200, None, &self.content),
            };
        Ok(WireResponse {
            status,
            location: None,
            content_range,
            body: Box::new(CannedBody {
                chunks: served.chunks(self.chunk).map(<[u8]>::to_vec).collect(),
            }),
        })
    }
}

struct CannedBody {
    chunks: VecDeque<Vec<u8>>,
}

#[async_trait]
impl FetchBody for CannedBody {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, FetchTransportError> {
        Ok(self.chunks.pop_front())
    }
}
