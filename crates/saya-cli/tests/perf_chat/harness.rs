//! The scripted-provider pieces the perf runs drive: the mock SSE provider's
//! request reader, the reply shapes the script names, and the server itself.
//!
//! The perf run builds its own child (it takes a binary path from
//! `SAYA_PERF_BIN`, not `CARGO_BIN_EXE_saya`), so the shared `SpawnedChild`
//! is not included; everything here is the mock server's own side.
//! This binary uses part of each file and marks that with `#[allow(dead_code)]`
//! on the module in `perf_chat.rs`, as the other included-harness binaries do.

include!("../support/scripted_provider/provider_io.rs");
include!("../support/scripted_provider/provider_responses.rs");
include!("../support/scripted_provider/scripted_server.rs");
