//! The `saya -p` adapter: open in place, stream the job, stop on Ctrl-C, and
//! print the one result object.
//!
//! The operation itself lives in [`super::oneshot`]; this file owns only the
//! process surface — the sink, the signal, and the terminal events.

use super::{
    CodingApplication, CodingError, LaunchPlan, OneShotRequest, cli::find_git, oneshot_finish,
    oneshot_start, root_mode::session_inputs,
};
use crate::{
    cli::RootMode,
    commands::output,
    config::runtime::RuntimeConfig,
    render::{RenderFormat, TerminalEvent},
    stream_render::TerminalSink,
};
use saya_agent::{AgentEventSink, CancellationToken, NoopEventSink};
use saya_store::SqliteStateStore;
use std::{io::Write as _, path::Path, time::Instant};

/// Runs one `-p` task to completion and returns the process exit code.
pub(crate) async fn run(
    request: OneShotRequest,
    plan: LaunchPlan,
    format: RenderFormat,
    runtime: RuntimeConfig,
) -> Result<i32, CodingError> {
    let (state_root, program_dir) = session_inputs(&runtime)?;
    let git = find_git()?;
    let store = SqliteStateStore::new(crate::state_path::state_db_path());
    let application = CodingApplication::open_default_with_mode(
        runtime,
        store,
        Path::new("."),
        &state_root,
        &git,
        &program_dir,
        RootMode::InPlace,
        &CancellationToken::new(),
    )
    .await?
    .with_approval_policy(plan.policy);

    let mode = request.mode;
    let job = oneshot_start(&application, request, &plan, CancellationToken::new()).await?;
    emit_started(&application, &job, mode, &plan, format);

    let started = Instant::now();
    // Ndjson streams every event; json and text keep stdout for the result
    // alone. Both read the same job.
    let sink: Box<dyn AgentEventSink> = match format {
        RenderFormat::Ndjson => Box::new(TerminalSink::new(RenderFormat::Ndjson)),
        RenderFormat::Text | RenderFormat::Json => Box::new(NoopEventSink),
    };
    let work = job.run(sink.as_ref());
    tokio::pin!(work);
    let result = tokio::select! {
        result = &mut work => result,
        _ = tokio::signal::ctrl_c() => {
            // Cancel first, then await the run so finalization records the
            // Cancelled state the result and exit code read.
            job.cancel();
            work.await
        }
    };

    let result = oneshot_finish(&application, &job, result, started.elapsed()).await;
    emit_result(result.value, format);
    let _ = std::io::stdout().flush();
    Ok(result.exit_code)
}

/// The stream's first line. Json keeps its one-object stdout contract, so it
/// states nothing here; text says `job <id>` on stderr.
fn emit_started(
    application: &CodingApplication,
    job: &super::NativeCodingJob,
    mode: super::OneShotMode,
    plan: &LaunchPlan,
    format: RenderFormat,
) {
    if matches!(format, RenderFormat::Json) {
        return;
    }
    output::emit(
        TerminalEvent::JobStarted {
            job_id: job.job_id().to_string(),
            cwd: application.root_path().display().to_string(),
            permission_mode: mode.as_str().to_owned(),
            write_paths: plan.write_paths.clone(),
        },
        format,
    );
}

/// The stream's last line: the result document. The library value carries its
/// own `event` key for `saya start`; the terminal event supplies the tag, so
/// it is removed here rather than serialized twice.
fn emit_result(mut value: serde_json::Value, format: RenderFormat) {
    if let Some(object) = value.as_object_mut() {
        object.remove("event");
    }
    output::emit(TerminalEvent::JobResult { value }, format);
}
