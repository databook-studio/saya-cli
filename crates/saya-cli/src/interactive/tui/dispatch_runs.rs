//! TUI adapter for the `/runs` family.
//!
//! The TUI runs under the alternate screen, so the shared `run_management`
//! dispatcher — which `emit`s to the thread-local seam — is wrapped here the
//! way `dispatch_contracts` wraps `run_contracts`: the dispatcher runs, and
//! its captured output is pushed into the transcript as a system or error
//! block. `/runs` and `/run cancel` are quick store reads and writes, so the
//! same blocking posture the contract commands have is right for them too.
//!
//! `/run <goal…>` is deliberately declined here (see `dispatch.rs`): a run's
//! stream writes to the real stdout, which the TUI does not own while the
//! alternate screen is up — the headless session or a shell hosts it.

use super::transcript::{BlockKind, Transcript};
use crate::cli::RunCommand;
use crate::commands::run_management;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session_resume::block_on;
use crate::render::RenderFormat;

/// Runs a run-management command through the shared dispatcher and pushes its
/// rendered output into the transcript — the same bytes the headless command
/// prints, captured via the seam instead of painted to the process stdout.
pub(super) fn run_management_command(
    transcript: &mut Transcript,
    runtime: &RuntimeConfig,
    state_db: &saya_store::SqliteStateStore,
    format: RenderFormat,
    command: RunCommand,
) {
    // Approval is only consulted by `resume`, which the session routes to the
    // nested child; a read or a cancel never prompts.
    let approval = saya_agent::ApprovalPolicy::ReadOnly;
    let work = run_management(command, runtime, format, approval, state_db);
    capture_and_push(transcript, work);
}

/// Awaits the dispatcher's future and pushes its captured output into the
/// transcript: the body on success (a system block), on failure the same body
/// as an error block — the exit code decides the block kind, the bytes stay
/// the headless bytes.
fn capture_and_push(
    transcript: &mut Transcript,
    work: impl std::future::Future<Output = Result<i32, Box<dyn std::error::Error>>>,
) {
    use crate::commands::{capture_output_start, capture_output_take};
    capture_output_start();
    let code = match block_on(work) {
        Ok(code) => code,
        Err(error) => {
            let _ = capture_output_take();
            transcript.push(BlockKind::Error, error.to_string());
            return;
        }
    };
    let (out, err) = capture_output_take();
    let body = if out.trim().is_empty() { err } else { out };
    if code == 0 {
        transcript.push(BlockKind::System, body.trim_end().to_string());
    } else {
        transcript.push(BlockKind::Error, body.trim_end().to_string());
    }
}
