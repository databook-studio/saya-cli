//! A session's cost ceiling, against the scripted provider.
//!
//! Two facts, both read off a real REPL child:
//!
//! 1. a provider that reports a cost at the ceiling stops the next turn before
//!    it sends a request, and `/usage` states the spend, the ceiling and the
//!    context share in its one panel;
//! 2. a provider that reports tokens but no cost never trips the ceiling: an
//!    unknown spend is not a spend of zero, and `/usage` says so.
//!
//! Memory is off in both rows, so no post-turn extraction adds a request the
//! script would have to answer.

#[cfg(unix)]
use std::{
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[cfg(unix)]
use crate::session_question_budget::harness::{
    ScriptStep, spawn_child_with_config_extra, spawn_script_server, wait_for_stream,
    wait_for_stream_count,
};

/// The one config both rows state: the operator's sharing consent, this
/// session's ceiling, and no memory extraction.
#[cfg(unix)]
fn config(ceiling_usd: &str) -> String {
    format!(
        "[ai]\nallow_data_sharing = true\n\n\
         [limits]\nsession_cost_usd = {ceiling_usd}\n\n\
         [memory]\nmode = 'off'\n"
    )
}

#[cfg(unix)]
fn listener() -> TcpListener {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    listener
}

#[cfg(unix)]
#[test]
fn the_ceiling_stops_the_next_turn_at_the_boundary() {
    let release = Arc::new(AtomicBool::new(false));
    let mut server = spawn_script_server(
        listener(),
        vec![ScriptStep::NaturalWithCost {
            input_tokens: 1_000,
            cost_usd: 0.5,
        }],
        release,
    );
    let mut spawned =
        spawn_child_with_config_extra(&server.address, &config("0.5"), "cost-ceiling", &[]);
    spawned.send(b"first question\n");
    wait_for_stream(&spawned.stdout_buf, b"the-final-answer", "the first answer");
    // The panel states the one request, the reported cost, the ceiling and the
    // context share: the spend and the ceiling are the same figure here.
    spawned.send(b"/usage\n");
    wait_for_stream(
        &spawned.stdout_buf,
        "Session usage — 1 request".as_bytes(),
        "the usage panel",
    );
    wait_for_stream(&spawned.stdout_buf, b"Cost: $0.5", "the reported cost");
    wait_for_stream(
        &spawned.stdout_buf,
        "Ceiling: $0.5 — $0.5 spent".as_bytes(),
        "the ceiling and the spend",
    );
    wait_for_stream(&spawned.stdout_buf, b"Context: ", "the context share");
    // The next request would start at the ceiling, so the turn stops instead.
    spawned.send(b"second question\n");
    wait_for_stream(
        &spawned.stdout_buf,
        b"Session cost ceiling reached",
        "the one-sentence stop",
    );
    spawned.send(b"/exit\n");
    spawned.finish();
    server.join();
    assert_eq!(
        server.requests.load(Ordering::SeqCst),
        1,
        "the stopped turn must not reach the provider"
    );
}

#[cfg(unix)]
#[test]
fn an_unreported_cost_never_stops_a_turn() {
    let release = Arc::new(AtomicBool::new(false));
    let mut server = spawn_script_server(
        listener(),
        vec![
            ScriptStep::NaturalWithUsage {
                input_tokens: 1_000,
            },
            ScriptStep::NaturalWithUsage {
                input_tokens: 2_000,
            },
        ],
        release,
    );
    let mut spawned =
        spawn_child_with_config_extra(&server.address, &config("0.01"), "cost-unreported", &[]);
    spawned.send(b"first question\n");
    wait_for_stream_count(
        &spawned.stdout_buf,
        b"the-final-answer",
        "answers",
        1,
        Duration::from_secs(30),
    );
    spawned.send(b"second question\n");
    wait_for_stream_count(
        &spawned.stdout_buf,
        b"the-final-answer",
        "answers",
        2,
        Duration::from_secs(30),
    );
    // The panel states the absence, and why an absence cannot trip the ceiling.
    spawned.send(b"/usage\n");
    wait_for_stream(
        &spawned.stdout_buf,
        b"Cost: not reported by the provider",
        "the unreported cost",
    );
    wait_for_stream(
        &spawned.stdout_buf,
        b"spend is not reported by the provider",
        "the ceiling's reason",
    );
    spawned.send(b"/exit\n");
    spawned.finish();
    server.join();
    assert_eq!(
        server.requests.load(Ordering::SeqCst),
        2,
        "an unreported cost must let both turns run"
    );
}
