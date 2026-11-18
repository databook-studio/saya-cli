//! The chat surface: the real REPL child over the shared scripted-provider
//! harness, with one SQLite profile configured and one plain message. The
//! first provider request is the turn's request, so its `tools` array is what
//! the model sees on turn one.

use std::{
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use super::{capture::Advertised, harness};

const QUESTION: &[u8] = b"what is the total revenue\n";
const ANSWER: &[u8] = b"the-final-answer";

pub(super) fn capture() -> Advertised {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut server = harness::spawn_script_server(
        listener,
        vec![harness::ScriptStep::Natural],
        Arc::new(AtomicBool::new(false)),
    );
    let root =
        std::env::temp_dir().join(format!("saya-run-core-parity-chat-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("parity.sqlite");
    std::fs::write(&database, []).unwrap();
    let connections = root.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\nread_only = true\n",
            database.display()
        ),
    )
    .unwrap();
    // Data sharing is the operator's explicit consent, memory is off so no
    // post-turn extraction adds a request, and the question budget is ample.
    let config = "[ai]\nallow_data_sharing = true\n\n\
                  [memory]\nmode = 'off'\n\n\
                  [limits.session]\nelapsed_seconds = 60\n";
    let mut child = harness::spawn_child_with_config_extra(
        &server.address,
        config,
        "parity",
        &[
            "--connections",
            connections.to_str().unwrap(),
            "--profile",
            "local",
            "--approval-mode",
            "bypass",
        ],
    );
    child.send(QUESTION);
    harness::wait_for_stream(&child.stdout_buf, ANSWER, "the chat answer");
    let deadline = Instant::now() + Duration::from_secs(30);
    while server.requests.load(Ordering::SeqCst) == 0 {
        assert!(
            Instant::now() < deadline,
            "the chat surface sent no provider request"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let body = server
        .bodies
        .lock()
        .unwrap()
        .first()
        .cloned()
        .expect("the chat surface sent a first request");
    child.send(b"/exit\n");
    child.finish();
    server.join();
    let _ = std::fs::remove_dir_all(&root);
    let value: serde_json::Value =
        serde_json::from_str(&body).expect("the provider request body is JSON");
    Advertised::from_body(&value)
}
