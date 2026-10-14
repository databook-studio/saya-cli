#![cfg(unix)]

#[path = "coding_runtime_contracts/command_output_capture.rs"]
mod output_capture;
#[allow(dead_code)] // shared test harness: this file uses only part of it
#[path = "coding_runtime_contracts/output_capture_support.rs"]
mod support;
