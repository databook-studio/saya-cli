//! Running the full-screen TUI session: setup, the draw tick, teardown.

use super::super::loop_tick::MouseCapture;
use super::super::session_save;
use super::super::terminal::TerminalGuard;
use super::super::ui;
use super::startup::build_app;
use super::{TrustOutcome, TuiSession};
use crate::interactive::session_prompt;
use crate::local_metrics::LocalMetrics;
use saya_agent::{AgentMetric, AgentMetricOutcome, AgentMetricPhase};
use std::sync::Arc;

/// Runs the full-screen TUI session. Returns the process exit code.
///
/// `trusted_echo` carries the startup trust prompt's echo — the moment of
/// choice names the just-trusted tree beside the bypass line's lane fact.
/// `None` on every path that did not trust. `trust_pending` opens the
/// startup trust modal once after the splash paints — the TUI's rendering
/// of the one trust decision, never a raw stdin read in front of the
/// interface. `launch` recomposes the universe behind a modal trust answer
/// so the bound session carries the launch's deny list and host statement.
pub(crate) fn run(args: TuiSession<'_>) -> Result<TrustOutcome, Box<dyn std::error::Error>> {
    let entered = std::time::Instant::now();
    let TuiSession {
        runtime,
        store,
        state_db,
        format,
        state,
        session,
        trusted_echo,
        trust_pending,
        launch,
        options,
    } = args;
    let choice = runtime.resolved.output_color;
    use std::io::IsTerminal as _;
    let is_terminal = std::io::stdout().is_terminal();
    let color_enabled =
        ui::theme::decide_public(choice, is_terminal, std::env::var_os("NO_COLOR").is_some());
    ui::theme::set_color_enabled(color_enabled);
    // Resolved before `TerminalGuard::new()` enables raw mode and the
    // alternate screen: `Auto` may query the terminal over OSC 11, and that
    // reply must land before anything reads stdin as key input, not race
    // the event loop that starts once the guard and app exist.
    ui::theme::set_theme(ui::theme::resolve_startup_theme(
        runtime.resolved.ui_theme,
        color_enabled,
        is_terminal,
        ui::theme::probe_terminal_theme,
        std::env::var("COLORFGBG").ok().as_deref(),
    ));
    let mut guard = TerminalGuard::new()?;
    let mut app = build_app(
        runtime,
        state_db,
        state,
        session,
        trusted_echo,
        trust_pending,
    );
    // Tracks the terminal's actual mouse-capture state; TerminalGuard enables it.
    let mut mouse = MouseCapture {
        captured: true,
        error_reported: false,
    };
    // A modal trust answer, when one binds a directory: the live runtime
    // recomposes behind the app's snapshot once the answer lands.
    let mut trusted_dir: Option<std::path::PathBuf> = None;
    let mut first_paint_recorded = false;

    while !app.should_quit {
        app.poll_session_picker();
        app.poll_investigation_picker();
        session_save::poll_session_save(&mut app, store, state);
        let status = session_prompt::status_segments(state);
        guard
            .terminal
            .draw(|frame| ui::draw(frame, &app, &status))?;
        if !first_paint_recorded {
            first_paint_recorded = true;
            record_first_paint(entered, session.session_metrics());
        }

        super::super::loop_tick::tick_events(
            &mut app,
            session,
            runtime,
            launch,
            state,
            &mut trusted_dir,
        )?;
        super::super::loop_tick::tick_mouse_capture(
            &mut app,
            guard.terminal.backend_mut(),
            &mut mouse,
        );
        super::super::loop_tick::tick_clipboard(&mut app, guard.terminal.backend_mut());
        super::super::loop_tick::tick_workers(&mut app, store, state);
        super::super::loop_tick::tick_busy(&mut app, store, state, session);
        // The queued-prompt drain, which owns the `/setup` handoff: the
        // guard is threaded through so the wizard can release and reacquire
        // the terminal modes. An io error here means the terminal could not
        // be re-entered — the `?` stops the loop and returns it from `run`
        // (the terminal is already restored; do not keep drawing).
        let mut setup = super::super::loop_tick::pending::SetupInputs {
            modes: &mut guard,
            options,
        };
        super::super::loop_tick::tick_pending(
            &mut app, state, runtime, store, format, session, &mut setup,
        )?;
        super::super::loop_tick::tick_resume(&mut app, store, state, runtime, session);
    }

    // Session teardown: remove the chart temp files this session wrote, before
    // the terminal state is restored (DESIGN §6.6). The piped-REPL and error
    // paths drain the same registry in `session_loop`.
    crate::chart::cleanup_session_charts();

    Ok(match trusted_dir {
        Some(dir) => TrustOutcome::Answered(dir),
        None => TrustOutcome::Unasked,
    })
}

fn record_first_paint(entered: std::time::Instant, metrics: Option<Arc<LocalMetrics>>) {
    if let Some(metrics) = metrics {
        metrics.record(&AgentMetric::Phase {
            phase: AgentMetricPhase::FirstPaint,
            elapsed_ns: u64::try_from(entered.elapsed().as_nanos()).unwrap_or(u64::MAX),
            outcome: AgentMetricOutcome::Succeeded,
        });
    }
}
