/// Runs the interactive session loop.
///
/// When attached to a terminal, each prompt is preceded by a one-line status
/// header (active profile, included databases, provider/model, approval mode,
/// workspace root, and privacy state) and the `saya> ` input marker. Normal
/// terminal scrollback is preserved. Piped input reads lines without the
/// status header, so scripts and CI behave predictably.
pub fn run(cli: Cli) -> Result<i32, Box<dyn std::error::Error>> {
    let ctx = prepare_launch(cli)?;
    if ctx.terminal_probe {
        run_terminal(ctx)
    } else {
        run_headless(ctx)
    }
}
