#![cfg(not(windows))]

//! End-to-end ApprovalWait phase proof: a verbose one-shot `ask` under a real
//! PTY records its approval wait sample in the printed local-metrics section
//! when the user answers the prompt, while the automatic path (`read-only`)
//! records none — the section keeps `unavailable`, and the fixed section
//! never carries prompt, SQL, or answer text.

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{Arc, Mutex, Once, mpsc},
    thread,
    time::{Duration, Instant},
};

const DEADLINE: Duration = Duration::from_secs(15);

#[test]
fn approval_wait_records_one_sample_when_the_user_answers() {
    warm_up_saya();
    let terminal = run_approval_metrics_case("answered", "ask", Some(b"a\r"));
    assert_metrics_section(&terminal, "phase approval_wait samples=1");
}

#[test]
fn automatic_approval_records_no_approval_wait_sample() {
    warm_up_saya();
    let terminal = run_approval_metrics_case("automatic", "read-only", None);
    assert_metrics_section(&terminal, "phase approval_wait unavailable");
}

include!("approval_wait_metrics_pty/case.rs");
include!("approval_wait_metrics_pty/pty_support.rs");
include!("approval_wait_metrics_pty/provider.rs");
