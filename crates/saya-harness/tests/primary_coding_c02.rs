#![cfg(unix)]

#[path = "primary_coding/c02.rs"]
mod c02;
#[allow(dead_code)] // shared test harness: this file uses only part of it
#[path = "primary_coding/c02_support.rs"]
mod support;
