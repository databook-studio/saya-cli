/// Runs the interactive session loop.
///
/// Without a terminal the plain line reader runs, printing no status header, so
/// scripts and CI behave predictably. On a terminal the launch opens the
/// full-screen TUI — unless it asked for the plain line REPL, by `--plain` or
/// `SAYA_PLAIN`: then the plain loop runs on the terminal too, with the
/// one-line status header (active profile, included databases, provider/model,
/// approval mode, workspace root, and privacy state) and the `saya> ` input
/// marker in front of each prompt, preserving normal terminal scrollback.
pub fn run(cli: Cli) -> Result<i32, Box<dyn std::error::Error>> {
    let ctx = prepare_launch(cli)?;
    let tui_surface = ctx.terminal_probe && !ctx.plain;
    if tui_surface {
        run_terminal(ctx)
    } else {
        run_headless(ctx)
    }
}
