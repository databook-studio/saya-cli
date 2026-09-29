//! The bounded stdio transport (ADR 0008, invariant 1): rmcp's own stdio
//! transport reads inbound lines with no size cap, so `serve` is handed this
//! transport instead. A reader task pushes every stdin byte through the
//! [`LineGate`] and delivers only completed lines that fit the request bound;
//! a discarded line is answered immediately by the single writer task, so
//! the refusal goes out even when the client sends nothing else. One writer
//! means protocol frames can never interleave.

use std::io;

use rmcp::{
    RoleServer,
    service::{RxJsonRpcMessage, TxJsonRpcMessage},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::mpsc,
};

use super::{
    line_gate::{LineAction, LineGate, oversized_line_error, parse_inbound},
    policy::MAX_REQUEST_BYTES,
};

/// Fixed read chunk: stdin is consumed in pieces of this size, so memory
/// stays at the cap plus this constant whatever the client sends.
const READ_CHUNK: usize = 8192;
/// Channel depth for frames waiting on the writer; bounded so a stalled
/// stdout cannot grow memory without limit.
const FRAME_CHANNEL_DEPTH: usize = 64;

type Inbound = RxJsonRpcMessage<RoleServer>;
type Outbound = TxJsonRpcMessage<RoleServer>;

pub(crate) struct BridgeTransport {
    outbound_tx: mpsc::Sender<Outbound>,
    inbound_rx: mpsc::Receiver<Inbound>,
}

impl BridgeTransport {
    /// Spawns the reader and writer tasks over raw stdio and returns the
    /// transport the rmcp service drives.
    pub(crate) fn new<R, W>(reader: R, writer: W) -> Self
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (inbound_tx, inbound_rx) = mpsc::channel(FRAME_CHANNEL_DEPTH);
        let (outbound_tx, outbound_rx) = mpsc::channel::<Outbound>(FRAME_CHANNEL_DEPTH);
        tokio::spawn(reader_task(reader, inbound_tx, outbound_tx.clone()));
        tokio::spawn(writer_task(writer, outbound_rx));
        Self {
            outbound_tx,
            inbound_rx,
        }
    }
}

impl rmcp::transport::Transport<RoleServer> for BridgeTransport {
    type Error = io::Error;

    fn send(
        &mut self,
        item: Outbound,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        let outbound_tx = self.outbound_tx.clone();
        async move {
            outbound_tx
                .send(item)
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "stdout writer is closed"))
        }
    }

    async fn receive(&mut self) -> Option<Inbound> {
        self.inbound_rx.recv().await
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

async fn reader_task<R>(
    mut reader: R,
    inbound: mpsc::Sender<Inbound>,
    outbound: mpsc::Sender<Outbound>,
) where
    R: AsyncRead + Unpin,
{
    let mut buf = vec![0u8; READ_CHUNK];
    let mut gate = LineGate::new(MAX_REQUEST_BYTES);
    loop {
        match reader.read(&mut buf).await {
            Ok(0) => {
                gate.finish();
                return;
            }
            Ok(n) => {
                // Actions are collected first and applied in order: deliver
                // the completed lines (inbound), then answer the discarded
                // ones (outbound). A response to any later line can only be
                // produced after its delivery, so the refusal still precedes
                // it on the wire.
                let mut accepted: Vec<Vec<u8>> = Vec::new();
                let mut refused = 0usize;
                gate.absorb(
                    &buf[..n],
                    &mut |line| accepted.push(line.to_vec()),
                    &mut || refused += 1,
                );
                for line in accepted {
                    match parse_inbound(&line) {
                        LineAction::Deliver(message) => {
                            if inbound.send(*message).await.is_err() {
                                return;
                            }
                        }
                        LineAction::Answer(error) => {
                            if outbound.send(Outbound::error(error, None)).await.is_err() {
                                return;
                            }
                        }
                        LineAction::Ignore => {}
                    }
                }
                for _ in 0..refused {
                    if outbound
                        .send(Outbound::error(oversized_line_error(), None))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
            Err(error) => {
                eprintln!("saya mcp: stdin read failed: {error}");
                return;
            }
        }
    }
}

async fn writer_task<W>(mut writer: W, mut outbound_rx: mpsc::Receiver<Outbound>)
where
    W: AsyncWrite + Unpin,
{
    while let Some(message) = outbound_rx.recv().await {
        let mut frame = match serde_json::to_vec(&message) {
            Ok(bytes) => bytes,
            Err(error) => {
                eprintln!("saya mcp: response frame could not be serialized: {error}");
                continue;
            }
        };
        frame.push(b'\n');
        if writer.write_all(&frame).await.is_err() || writer.flush().await.is_err() {
            eprintln!("saya mcp: stdout write failed; the writer task exits");
            return;
        }
    }
}
