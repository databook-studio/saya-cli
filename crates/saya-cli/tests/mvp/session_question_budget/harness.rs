//! Shared harness for the scripted-provider REPL rows: the child guard,
//! the mock SSE provider helpers, the output pumps, and a scriptable mock
//! server. The question-budget rows and the line front's end-to-end rows both
//! include it.
//!
//! The pieces live under `tests/support/scripted_provider/` and hold no
//! Unix-only call — threads, TCP streams, piped stdio, and temp paths are all
//! plain `std` — so the same bytes compile for a Windows test run. A binary
//! that includes this module uses only part of it and marks that with
//! `#[allow(dead_code)]` on the module.

include!("../../support/scripted_provider/provider_io.rs");
include!("../../support/scripted_provider/stream_wait.rs");
include!("../../support/scripted_provider/provider_responses.rs");
include!("../../support/scripted_provider/scripted_server.rs");
include!("../../support/scripted_provider/spawned_child.rs");
