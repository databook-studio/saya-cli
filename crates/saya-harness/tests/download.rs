//! The `http_download` battery, served entirely from `127.0.0.1` — no test
//! touches the real network; DNS is a fake table.
//!
//! The plan-required cases: the budget trip that pauses fail-safe and leaves
//! a resumable partial, the killed download that resumes to the whole
//! content's digest, the tampered partial refused on digest, the refused
//! redirect hop writing nothing, the resolved private address refused before
//! any connection (the listener is the witness), the per-file and per-run
//! byte bounds each tripping typed, and the contained destination with an
//! unchanged sentinel outside the workspace. Two loop-level tests prove the
//! declaration: the default denies `http_download` and no file appears.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentLimits, AgentRequest, CancellationToken, ChatProvider,
    ChatRequest, ChatResponse, ProviderError, ProviderEvent, ProviderStream, ToolCall, ToolError,
    ToolExecutor, run_agent_with_sink,
};
use saya_harness::HarnessError;
use saya_harness::fetch::{
    DownloadBudget, DownloadCall, DownloadError, DownloadLimits, FetchBody, FetchDestination,
    FetchPolicy, FetchRefusal, FetchRequest, FetchTransport, FetchTransportError, WireResponse,
    http_download, http_download_cancellable, http_download_definition,
};
use saya_harness::workspace::Workspace;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[path = "download/support.rs"]
mod support;
