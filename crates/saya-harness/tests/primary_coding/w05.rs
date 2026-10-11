#![cfg(any(target_os = "macos", target_os = "linux"))]

#[path = "w05/create.rs"]
mod create;
#[path = "w05/create_edges.rs"]
mod create_edges;
#[path = "w05/create_retry.rs"]
mod create_retry;
#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)] // shared test harness: this file uses only part of it
#[path = "w05/create_support.rs"]
mod create_support;
#[path = "w05/ownership.rs"]
mod ownership;
#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)]
#[path = "w02_support.rs"]
mod support;
#[path = "w05/worktree.rs"]
mod worktree;
