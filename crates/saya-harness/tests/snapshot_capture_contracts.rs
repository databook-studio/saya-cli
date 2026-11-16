#![cfg(any(target_os = "macos", target_os = "linux"))]

#[path = "coding_runtime_contracts/snapshot_capture.rs"]
mod snapshot_capture;
#[allow(dead_code)] // shared test harness: this file uses only part of it
#[path = "coding_runtime_contracts/worktree_inventory_support.rs"]
mod support;
