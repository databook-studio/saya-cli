//! The `saya sessions list|delete` clap subcommand handler: it maps the
//! shared operations' typed outcomes through the render helpers and the
//! CLI's output channel — no behavior of its own, no prompting, no stdin.

use super::{
    EXIT_SESSIONS_ERROR, delete_saved_session, list_sessions, render_delete, render_listing,
};
use crate::cli::SessionsCommand;
use crate::commands::output::{failure_message, result};
use crate::interactive::session_paths::default_session_dir;
use crate::render::RenderFormat;
use saya_store::FsSessionStore;

pub(crate) async fn run(
    command: SessionsCommand,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    match command {
        SessionsCommand::List => list(format).await,
        SessionsCommand::Delete { id } => delete(&id, format),
    }
}

async fn list(format: RenderFormat) -> Result<i32, Box<dyn std::error::Error>> {
    let sessions_root = default_session_dir();
    let store = FsSessionStore::new(&sessions_root);
    match list_sessions(&store).await {
        Ok(listing) => result(
            render_listing(&listing, super::operations::unix_ms_now(), &sessions_root),
            format,
        ),
        Err(error) => failure_message(
            EXIT_SESSIONS_ERROR,
            format!("could not list saved sessions: {error}"),
            format,
        ),
    }
}

fn delete(id: &str, format: RenderFormat) -> Result<i32, Box<dyn std::error::Error>> {
    let sessions_root = default_session_dir();
    let store = FsSessionStore::new(&sessions_root);
    let outcome = delete_saved_session(&store, &sessions_root, id, None);
    let (code, message) = render_delete(&outcome, id);
    if code == 0 {
        result(message, format)
    } else {
        failure_message(code, message, format)
    }
}
