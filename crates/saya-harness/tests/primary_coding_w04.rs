#![cfg(any(target_os = "macos", target_os = "linux"))]

#[allow(dead_code)] // shared test harness: this file uses only part of it
#[path = "primary_coding/w02_support.rs"]
mod support;
#[path = "primary_coding/w04.rs"]
mod w04;
