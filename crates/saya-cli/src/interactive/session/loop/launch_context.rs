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
    trusted_echo: Option<String>,
    tui_trust_pending: bool,
    launch: crate::interactive::session_host::HostLaunch,
}
