//! The scripted multi-round mock provider and scratch environment for the
//! turn-file coding tests: one scripted SSE reply per accepted request, each
//! round computed from the request body it answers, every raw request
//! captured for assertions.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

include!("harness/provider.rs");
include!("harness/environment.rs");
include!("harness/run.rs");
include!("harness/requests.rs");
#[cfg(unix)]
include!("harness/background.rs");
