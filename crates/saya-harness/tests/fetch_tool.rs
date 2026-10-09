//! The `http_fetch` tool battery, written red-first against the fetch slice.
//!
//! The five plan-required cases: the hostile-body breakout (the context lane),
//! the adversarial-download invariance (§10), the refused-redirect hop whose
//! body is never read, the resolved-private-address refusal on an allowlisted
//! name, and the three bounds tripping as typed errors. Every test is served
//! by a real HTTP listener on `127.0.0.1` — no test touches the real network;
//! DNS is a fake table and names that must not resolve are `.invalid`.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use saya_agent::build_messages;
use saya_harness::fetch::{
    FetchDestination, FetchLimits, FetchPolicy, FetchRefusal, FetchToolError, HttpFetchTool,
    http_fetch_definition,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[path = "fetch_tool/context_content.rs"]
mod context_content;
#[path = "fetch_tool/declared_paths.rs"]
mod declared_paths;
#[path = "fetch_tool/fetch_bounds.rs"]
mod fetch_bounds;
#[path = "fetch_tool/fixtures.rs"]
mod fixtures;
#[path = "fetch_tool/network_policy.rs"]
mod network_policy;
#[path = "fetch_tool/site.rs"]
mod site;
#[path = "fetch_tool/transport.rs"]
mod transport;
use fixtures::*;
use site::*;
use transport::*;
