//! Shared harness for the REPL question-budget rows: the child guard,
//! the mock SSE provider helpers, the output pumps, and a scriptable mock
//! server. Row B (idle wait) and Row A (remaining budget) share it.

include!("harness/provider_io.rs");
include!("harness/stream_wait.rs");
include!("harness/provider_responses.rs");
include!("harness/scripted_server.rs");
include!("harness/spawned_child.rs");
