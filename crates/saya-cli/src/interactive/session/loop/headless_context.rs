/// Reads one turn verbatim from `path`: the bytes reach the turn unaltered —
/// blank lines, trailing whitespace, code fences. Only a trailing `\n` or
/// `\r\n` (the file's own line ending) is stripped, since the per-turn entry
/// below treats a trailing newline as the end of input, not content.
fn read_turn_file(path: &std::path::Path) -> Result<String, String> {
    let mut text = crate::commands::query_input::read_file_bounded(
        path,
        crate::commands::query_input::FILE_BYTE_LIMIT,
    )
    .map_err(|error| match error {
        crate::commands::query_input::InputReadError::OverLimit { limit } => {
            format!("--turn-file exceeds the {limit}-byte limit; use a smaller turn")
        }
        crate::commands::query_input::InputReadError::Io(error) => {
            format!("cannot read --turn-file {}: {error}", path.display())
        }
        crate::commands::query_input::InputReadError::Utf8(error) => {
            format!("--turn-file {} is not valid UTF-8: {error}", path.display())
        }
        crate::commands::query_input::InputReadError::Idle => {
            format!(
                "cannot read --turn-file {}: input went idle",
                path.display()
            )
        }
    })?;
    if text.ends_with("\r\n") {
        text.truncate(text.len() - 2);
    } else if text.ends_with('\n') {
        text.truncate(text.len() - 1);
    }
    if text.trim().is_empty() {
        return Err(format!("--turn-file {} is empty", path.display()));
    }
    Ok(text)
}

/// The outcome of the single `--turn-file` turn: completed (exit 0),
/// errored (exit 5, the streamed `TerminalEvent::Error` names it), or an
/// explicit `/exit` (still exit 0 — the turn ran and asked to leave).
#[derive(Debug)]
enum TurnOutcome {
    Completed,
    Errored,
    Exit,
}

/// Everything one headless turn runs with. A bundle rather than seven
/// positional parameters, so the call sites read by name. `state` is the
/// session being turned; `session` is its engine side (universe, policy,
/// journal). `terminal` decides whether the loop prints the status header
/// and whether turns may prompt.
struct TurnContext<'a> {
    state: &'a mut SessionState,
    /// An owned handle so a `/reload` can swap it between lines: the swap
    /// is observed by the NEXT line, never mid-turn. Every line re-clones
    /// it into the `&RuntimeConfig` its consumers take.
    runtime: Arc<RuntimeConfig>,
    store: &'a FsSessionStore,
    state_db: &'a SqliteStateStore,
    format: RenderFormat,
    terminal: bool,
    session: &'a mut SessionRuntime,
    /// The trust answer's echo, said once at startup where the headless loop
    /// says its other startup facts. `None` on every path that did not trust.
    trusted_echo: Option<String>,
    /// The settled prose turn's summary — its status and the live sink's
    /// tool tally — for the scripted run's one terminal result. `None` for
    /// slash-command turns (no prose turn ran).
    last_turn: Option<TurnSummary>,
    /// Whether a save has already been refused (and said its friendly
    /// sentence) in this run: the scripted run's end-of-run save must not
    /// repeat the identical refusal the per-turn save reported.
    save_refused: bool,
    /// The launch's CLI options, cloned once: `/reload` re-reads the
    /// configuration with the SAME launch options via
    /// `ReloadInputs::from_launch`.
    launch_options: GlobalOptions,
}
