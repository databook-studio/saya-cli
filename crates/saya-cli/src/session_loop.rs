use super::{
    session_commands::SessionAction,
    session_state::{SessionLine, SessionState},
};
use crate::{
    Cli, config_runtime,
    render::{RenderFormat, TerminalEvent, render_event},
    slash::parse_slash_command,
};
use saya_store::{FsSessionStore, SessionStore};
use std::{
    io::{self, IsTerminal, Write},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

pub fn run(cli: Cli) -> Result<i32, Box<dyn std::error::Error>> {
    let runtime = config_runtime::load(&cli.options, std::path::Path::new("."))?;
    let format = config_runtime::format_name(&cli.options, &runtime.resolved);
    let store = FsSessionStore::new(session_dir());
    let mut state = load_session(&store, &cli)?;
    state.provider = runtime.resolved.ai.provider.as_str().into();
    state.model = runtime.resolved.ai.model.clone();
    state.allow_data_sharing = runtime.resolved.ai.allow_data_sharing;
    state.approval_mode = config_runtime::approval_name(&cli.options)?;
    state.included_profiles = cli.options.include_profiles.clone();
    let terminal = io::stdin().is_terminal();
    let mut input = String::new();
    loop {
        if terminal {
            print!("saya> ");
            io::stdout().flush()?;
        }
        input.clear();
        if io::stdin().read_line(&mut input)? == 0 {
            break;
        }
        let line = input.trim_end();
        if line.is_empty() {
            continue;
        }
        let action = match parse_slash_command(line)? {
            Some(command) => state.apply(
                command,
                &runtime
                    .connections
                    .profiles
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>(),
            ),
            None => {
                state.record("user", line);
                SessionAction::NotImplemented("interactive AI/database execution".into())
            }
        };
        if matches!(action, SessionAction::Exit) {
            break;
        }
        emit_action(action, format, &mut state);
        block_on(store.save(state.redacted()))?;
    }
    block_on(store.save(state.redacted()))?;
    Ok(0)
}

fn emit_action(action: SessionAction, format: RenderFormat, state: &mut SessionState) {
    match action {
        SessionAction::Message(message) => {
            state.record("system", &message);
            emit(TerminalEvent::Result { message }, format);
        }
        SessionAction::NotImplemented(feature) => {
            emit(TerminalEvent::NotImplemented { feature }, format)
        }
        SessionAction::Exit => {}
    }
}

fn load_session(
    store: &FsSessionStore,
    cli: &Cli,
) -> Result<SessionState, Box<dyn std::error::Error>> {
    let loaded = if let Some(id) = cli.options.resume.as_deref() {
        block_on(store.load(id))?
    } else if cli.options.continue_session {
        block_on(store.most_recent())?
    } else {
        None
    };
    match loaded {
        Some(value) => {
            let mut state = SessionState::new(
                value.id,
                value.profile_names.first().cloned(),
                "qwen2.5-coder:14b",
            );
            state.included_profiles = value.profile_names.into_iter().skip(1).collect();
            state.messages = value
                .messages
                .into_iter()
                .map(|line| SessionLine {
                    role: line.role,
                    content: line.content,
                })
                .collect();
            Ok(state)
        }
        None if cli.options.resume.is_some() || cli.options.continue_session => {
            Err("requested session was not found".into())
        }
        None => Ok(SessionState::new(
            new_id(),
            cli.options.profile.clone(),
            "qwen2.5-coder:14b",
        )),
    }
}

fn emit(event: TerminalEvent, format: RenderFormat) {
    let rendered = render_event(&event, format);
    print!("{}", rendered.stdout);
    eprint!("{}", rendered.stderr);
}
fn new_id() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis().to_string())
        .unwrap_or_else(|_| "session".into())
}
fn session_dir() -> PathBuf {
    std::env::var_os("SAYA_SESSION_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".saya/sessions"))
}
fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(future)
}
