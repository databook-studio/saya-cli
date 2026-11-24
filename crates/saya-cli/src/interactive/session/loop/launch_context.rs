/// The launch context one session is prepared into, and the one surface
/// decision it carries.
struct LaunchContext {
    cli: Cli,
    runtime: RuntimeConfig,
    store: FsSessionStore,
    state_db: SqliteStateStore,
    format: RenderFormat,
    state: SessionState,
    session: SessionRuntime,
    fresh: bool,
    terminal_probe: bool,
    /// The launch asked for the plain line REPL (`--plain` or `SAYA_PLAIN`):
    /// on a terminal it keeps the line front instead of the full-screen TUI.
    plain: bool,
    trusted_echo: Option<String>,
    tui_trust_pending: bool,
    launch: crate::interactive::session_host::HostLaunch,
}

/// Whether this launch asks for the plain line REPL: the `--plain` flag, or
/// the same request stated as `SAYA_PLAIN` in the environment. Read once, at
/// launch — nothing here or later reads either again, so the two spellings
/// cannot disagree about the surface one process opens.
fn plain_requested(options: &GlobalOptions) -> bool {
    options.plain || says_plain(std::env::var_os("SAYA_PLAIN"))
}

/// The environment spelling: `1`, `true`, `yes`, or `on` — case-insensitive —
/// states the plain REPL. Anything else, and an unset variable, states
/// nothing, so a stray `SAYA_PLAIN=0` cannot switch a TUI launch away from it.
fn says_plain(value: Option<std::ffi::OsString>) -> bool {
    value.is_some_and(|value| {
        matches!(
            value.to_string_lossy().trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}
