use super::{
    session_commands::SessionAction,
    session_request::PromptResult,
    session_resume::{SessionDefaults, block_on, load_session},
};
use crate::session_paths::default_session_dir;
use crate::{Cli, config_runtime, slash::parse_slash_command};
use saya_store::{FsSessionStore, SessionStore};
use std::io::{self, IsTerminal, Write};

pub fn run(cli: Cli) -> Result<i32, Box<dyn std::error::Error>> {
    let runtime = config_runtime::load(&cli.options, std::path::Path::new("."))?;
    let format = config_runtime::format_name(&cli.options, &runtime.resolved);
    let store = FsSessionStore::new(default_session_dir());
    let state_db = saya_store::SqliteStateStore::new(crate::state_path::state_db_path());
    let defaults = SessionDefaults {
        provider: runtime.resolved.ai.provider.as_str().into(),
        model: runtime.resolved.ai.model.clone(),
        allow_data_sharing: runtime.resolved.ai.allow_data_sharing,
        approval_mode: config_runtime::approval_name(&cli.options)?,
    };
    let mut state = load_session(&store, &cli, &defaults)?;
    if !cli.options.continue_session && cli.options.resume.is_none() {
        state.provider = runtime.resolved.ai.provider.as_str().into();
        state.model = runtime.resolved.ai.model.clone();
        state.allow_data_sharing = runtime.resolved.ai.allow_data_sharing;
        state.approval_mode = config_runtime::approval_name(&cli.options)?;
        state.included_profiles = cli.options.include_profiles.clone();
    }
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
                let history = state.provider_history();
                let approval = state
                    .approval_mode
                    .parse()
                    .map_err(|error: saya_agent::ApprovalPolicyParseError| error.to_string())?;
                match block_on(super::session_request::run(
                    &runtime,
                    line,
                    approval,
                    terminal,
                    state.prompt_overrides(),
                    history,
                    format,
                    &state_db,
                )) {
                    Ok(PromptResult::Completed(output)) => {
                        state.record_turn(
                            line,
                            output.answer.clone(),
                            output.used_bounded_sql_query,
                            output.tool_metadata.clone(),
                        );
                        SessionAction::Agent(output)
                    }
                    Ok(PromptResult::Cancelled) => SessionAction::Cancelled,
                    Err(error) => SessionAction::Error(error.to_string()),
                }
            }
        };
        if matches!(action, SessionAction::Exit) {
            break;
        }
        if let SessionAction::Schema(refresh) = &action {
            block_on(super::session_schema::run(
                &runtime,
                state.profile.as_deref(),
                *refresh,
                terminal,
                format,
                &state_db,
            ))?;
            block_on(store.save(state.redacted()))?;
            continue;
        }
        super::session_emit::emit_action(action, format, &mut state, &store)?;
        block_on(store.save(state.redacted()))?;
    }
    block_on(store.save(state.redacted()))?;
    Ok(0)
}
