#![cfg(any(target_os = "macos", target_os = "linux"))]

#[path = "w07/delta.rs"]
mod delta;
#[path = "w07/delta_edges.rs"]
mod delta_edges;
#[path = "w07/delta_output.rs"]
mod delta_output;
#[path = "w07/paths.rs"]
mod paths;
#[path = "w07/receipt.rs"]
mod receipt;

#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)]
#[path = "w05/create_support.rs"]
mod create_support;

#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)]
#[path = "w02_support.rs"]
mod support;
