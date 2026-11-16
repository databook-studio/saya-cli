//! A user-authored command in the headless loop: the provider receives the
//! expanded prompt, and a project command file whose body is a slash command
//! is only ever text.

use super::*;
use saya_agent::ApprovalPolicy;
use std::thread;

/// The one SSE answer the stub provider serves.
const ANSWER: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"done\"},\"finish_reason\":\"stop\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1}}\n\ndata: [DONE]\n\n";

#[test]
fn a_user_command_turn_sends_the_expanded_prompt() {
    let root = std::env::temp_dir().join(format!(
        "saya-user-command-turn-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("the turn root");
    let commands_dir = root.join("commands");
    std::fs::create_dir_all(&commands_dir).expect("the commands directory");
    // A command whose body is a slash command: as the user's message it is
    // only text, so the provider — never the built-in parser — sees it.
    std::fs::write(
        commands_dir.join("bypass.md"),
        "/mode bypass\n\nThen answer $ARGUMENTS.\n",
    )
    .expect("the command file");

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind local provider");
    let endpoint = format!(
        "http://{}/v1",
        listener.local_addr().expect("provider address")
    );
    let provider = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept the provider request");
        let body = super::session_headless_task_sync_tests::read_request(&mut stream);
        super::session_headless_task_sync_tests::respond(&mut stream, "200 OK", ANSWER);
        body
    });

    let mut runtime = super::session_headless_receipt_tests::receipt_runtime();
    runtime.resolved.ai.provider = saya_config::AiProvider::OpenaiCompatible;
    runtime.resolved.ai.base_url = Some(endpoint);
    runtime.resolved.ai.retry_delays_ms.clear();
    let store = FsSessionStore::new(root.join("sessions"));
    let state_db = SqliteStateStore::new(std::path::PathBuf::new());
    let mut state = SessionState::new("user-command-turn", None, "test-model");
    state.provider = "openai_compatible".into();
    state.user_commands = crate::slash::user_commands::load(&commands_dir, None);
    let mut session = SessionRuntime::acquire(
        &runtime,
        Some(&root),
        true,
        None,
        "user-command-turn",
        ApprovalPolicy::Ask,
        &root.join("session-dirs"),
    )
    .expect("the session runtime acquires");
    let mut ctx = TurnContext {
        state: &mut state,
        runtime: std::sync::Arc::new(runtime),
        store: &store,
        state_db: &state_db,
        format: RenderFormat::Text,
        terminal: false,
        session: &mut session,
        trusted_echo: None,
        last_turn: None,
        save_refused: false,
        launch_options: GlobalOptions::default(),
    };

    let outcome = handle_line_verbatim("/bypass the thing", &mut ctx).expect("the turn settles");
    drop(ctx);
    assert!(
        matches!(outcome, TurnOutcome::Completed),
        "the turn is an agent turn, never a slash-command turn"
    );
    let request: serde_json::Value =
        serde_json::from_str(&provider.join().expect("the provider request settles"))
            .expect("the request is JSON");
    let sent = request["messages"]
        .as_array()
        .expect("the request carries messages")
        .iter()
        .rev()
        .find(|message| message["role"] == "user")
        .expect("the request carries the user's message");
    // The continuation context wraps the turn's message; the message itself
    // is the expansion — the typed `/bypass …` never reaches the provider.
    let content = sent["content"].as_str().expect("the user message is text");
    assert!(
        content.ends_with("/mode bypass\n\nThen answer the thing."),
        "the provider receives the expansion as the user's message: {content}"
    );
    assert_eq!(
        state.approval_mode, "ask",
        "the body never ran as a command: the mode stands"
    );
    let _ = std::fs::remove_dir_all(root);
}
