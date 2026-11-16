use crate::{
    config::runtime::RuntimeConfig,
    interactive::sql_operation,
    render::{RenderFormat, TerminalEvent, render_event},
};

/// Executes a raw SQL query against the session's currently-active profile.
pub(crate) async fn run(
    runtime: &RuntimeConfig,
    profile_name: Option<&str>,
    sql: &str,
    can_prompt: bool,
    format: RenderFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    match sql_operation::execute(runtime, profile_name, sql, can_prompt).await {
        Ok(result) => emit(TerminalEvent::QueryResult { result }, format),
        Err(error) => emit(
            TerminalEvent::Error {
                message: error.to_string(),
            },
            format,
        ),
    }
    Ok(())
}

fn emit(event: TerminalEvent, format: RenderFormat) {
    let rendered = render_event(&event, format);
    print!("{}", rendered.stdout);
    eprint!("{}", rendered.stderr);
}
