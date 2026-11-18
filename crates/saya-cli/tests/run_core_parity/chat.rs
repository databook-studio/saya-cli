//! The chat surface: the real REPL child over the shared scripted-provider
//! harness, with one SQLite profile configured and one plain message. The
//! first provider request is the turn's request, so its `tools` array is what
//! the model sees on turn one.
//!
//! The script makes the model `read` one small file the fixture writes into
//! the child's working directory, so a tool result follows and a SECOND
//! request arrives; the request baseline compares the two prefixes.

use std::{
    net::TcpListener,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use super::{capture::Advertised, harness};

const QUESTION: &[u8] = b"what is the total revenue\n";
const ANSWER: &[u8] = b"the-final-answer";
/// The file the scripted `read` opens, and the body it finds there. The call
/// names it by a relative path, so the request carries no machine prefix.
const SAMPLE: &str = "small.txt";
const SAMPLE_BODY: &[u8] = b"revenue sample\n";
/// The requests the script produces: the question, then the turn that carries
/// the tool result back to the model.
const REQUESTS: usize = 2;

/// The chat surface's provider request bodies in arrival order, plus the
/// per-run roots those bodies may name.
pub(super) struct Conversation {
    pub(super) bodies: Vec<String>,
    pub(super) roots: Vec<PathBuf>,
}

pub(super) fn capture() -> Advertised {
    let conversation = converse();
    let first = conversation
        .bodies
        .first()
        .expect("the chat surface sent a first request");
    let value: serde_json::Value =
        serde_json::from_str(first).expect("the provider request body is JSON");
    Advertised::from_body(&value)
}

pub(super) fn converse() -> Conversation {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut server = harness::spawn_script_server(
        listener,
        vec![
            harness::ScriptStep::ToolCall {
                name: "read".into(),
                arguments: serde_json::json!({ "path": SAMPLE }),
            },
            harness::ScriptStep::Natural,
        ],
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
    // The child's cwd is its workspace root; write the sample there once the
    // harness has created the per-run tree.
    std::fs::write(child.config_root().join("cwd").join(SAMPLE), SAMPLE_BODY).unwrap();
    child.send(QUESTION);
    harness::wait_for_stream(&child.stdout_buf, ANSWER, "the chat answer");
    let deadline = Instant::now() + Duration::from_secs(30);
    while server.requests.load(Ordering::SeqCst) < REQUESTS {
        assert!(
            Instant::now() < deadline,
            "the chat surface sent fewer than {REQUESTS} provider requests"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let bodies = server.bodies.lock().unwrap().clone();
    // Read the per-run roots before `finish` removes them: the baseline
    // normalises them out of the bodies it measured.
    let roots = vec![
        root.clone(),
        child.config_root().to_path_buf(),
        child.session_root().to_path_buf(),
    ];
    child.send(b"/exit\n");
    child.finish();
    server.join();
    let _ = std::fs::remove_dir_all(&root);
    Conversation { bodies, roots }
}
