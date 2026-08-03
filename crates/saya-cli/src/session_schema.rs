use crate::{
    config_runtime::RuntimeConfig,
    render::{RenderFormat, TerminalEvent, render_event},
};
use saya_store::SqliteStateStore;

pub(crate) async fn run(
    runtime: &RuntimeConfig,
    profile: Option<&str>,
    refresh: bool,
    can_prompt: bool,
    format: RenderFormat,
    store: &SqliteStateStore,
) -> Result<(), Box<dyn std::error::Error>> {
    let name = profile.or(runtime.resolved.profile_name.as_deref());
    let Some(name) = name else {
        emit(
            TerminalEvent::Error {
                message: "Schema requires a selected profile.".into(),
            },
            format,
        );
        return Ok(());
    };
    let _ =
        crate::commands::connection_schema::run(name, refresh, runtime, format, can_prompt, store)
            .await?;
    Ok(())
}

fn emit(event: TerminalEvent, format: RenderFormat) {
    let rendered = render_event(&event, format);
    print!("{}", rendered.stdout);
    eprint!("{}", rendered.stderr);
}
