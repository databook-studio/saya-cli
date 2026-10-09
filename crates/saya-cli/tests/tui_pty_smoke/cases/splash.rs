#[test]
fn tui_paints_splash_and_status_bar_on_a_real_pty() {
    let home = scratch_home("off");
    // Clean up the scratch HOME no matter how this test ends — success, a
    // failed assertion, or a timeout panic. It never touches the developer's
    // real HOME or any real saya state.
    let _cleanup = HomeGuard(home.clone());

    let screen = match paint_screen(&home, Memory::Off) {
        Ok(screen) => screen,
        Err(reason) => {
            // Could not allocate a pty (e.g. a constrained CI runner). Skip
            // cleanly rather than fail — the packet requires this.
            eprintln!("skipping tui_pty_smoke: {reason}");
            return;
        }
    };
    assert_splash_and_status(&screen);
}

/// Memory-on startup path (packet P2b, defect #51). The recorded memory demos
/// run `[memory] mode = "assisted"` and the demo tapes captured the REPL
/// exiting at startup with `Error: local state store is unavailable`, the TUI
/// never painting. This is the cheapest rung — memory on, an empty scratch
/// HOME, nothing else — so it isolates the suspect path (eager store open at
/// startup under memory-on) from any pre-existing store state.
///
/// If this passes, #51 did not reproduce at rung 1: an empty HOME with memory on
/// paints the splash and stays up. That is a coverage win, not a failure — say
/// so explicitly, and do not weaken an assertion to make anything pass. If it
/// fails, the captured screen is the reproduction; land it `#[ignore]`d with
/// the exact condition and return a dependency request (the cause is almost
/// certainly outside saya-cli).
#[test]
fn tui_paints_splash_with_memory_on_empty_home() {
    let home = scratch_home("mem-on-empty");
    let _cleanup = HomeGuard(home.clone());

    let screen = match paint_screen(&home, Memory::Assisted) {
        Ok(screen) => screen,
        Err(reason) => {
            eprintln!("skipping tui_pty_smoke (memory on): {reason}");
            return;
        }
    };
    assert_splash_and_status(&screen);
}

/// Memory-on after a prior store write that needs no database (rung 2). A
/// first `saya` launch reaches the splash, which opens the state store at
/// startup (`reload_at_refs` → `get_schema` → `pool()` with
/// `create_if_missing`), writing `state.sqlite3` plus its `-wal`/`-shm`
/// sidecars. The second launch — same scratch HOME, so the store already
/// exists with sidecars — is the suspect path: a sibling process's WAL
/// checkpoint or a stale sidecar is exactly the condition packet 50's
/// bounded-open fix was written for, and the demos ran `connection schema
/// --refresh` (a store write) right before the REPL launch.
///
/// The first launch is asserted too: it must paint the splash (the rung-1
/// condition, in-place). Then the second launch is asserted against the same
/// positive/negative set. If either fails, the captured screen is the
/// reproduction.
#[test]
fn tui_paints_splash_with_memory_on_after_a_store_write() {
    let home = scratch_home("mem-on-storewrite");
    // One cleanup for the whole rung: the second launch must see the first
    // launch's store, so we do NOT clean between them.
    let _cleanup = HomeGuard(home.clone());

    let first = match paint_screen(&home, Memory::Assisted) {
        Ok(screen) => screen,
        Err(reason) => {
            eprintln!("skipping tui_pty_smoke (memory on, store write): {reason}");
            return;
        }
    };
    assert_splash_and_status_named(&first, "first launch");

    // The first launch opened the store; prove it before asserting the second
    // launch is meaningfully different. If the store was never created, rung 2
    // collapses into rung 1 and the finding is "store write did not happen".
    let state_db = home.join("state.sqlite3");
    if !state_db.exists() {
        panic!(
            "rung 2 precondition failed: first launch did not create {state_db:?} \
             (reload_at_refs may no longer open the store at startup); rung 2 \
             collapses into rung 1"
        );
    }

    let second = paint_screen(&home, Memory::Assisted)
        .expect("second launch pty allocation failed after the first succeeded");
    assert_splash_and_status_named(&second, "second launch (after store write)");
}

/// Memory-on after a real `connection schema --refresh` against the local
/// docker pagila (rung 3). This is the exact demo sequence — `connection
/// schema <profile> --refresh` then a REPL launch — and the most expensive
/// rung because the refresh needs postgres. It **skips cleanly** (not fails)
/// when the database is unreachable, so the suite stays green on a machine
/// without the `databook-postgres` container.
///
/// The refresh is a real store write against a live database (it populates
/// the schema cache the REPL's `reload_at_refs` then reads at startup). If the
/// defect is in the interaction of a freshly-written store and the REPL's
/// startup open — the demos' exact condition — this is the rung that would
/// surface it. The status bar shows `docker_postgres · …`, not `demo · …`.
#[test]
fn tui_paints_splash_with_memory_on_after_schema_refresh() {
    // Cheapest reachability probe: a TCP connect to the docker postgres port.
    // No docker CLI, no .env.saya, no provider key — just the port the
    // container would listen on. Unreachable ⇒ skip, never fail.
    const PAGILA: &str = "127.0.0.1:5434";
    if !tcp_reachable(PAGILA, Duration::from_secs(1)) {
        eprintln!("skipping rung 3: {PAGILA} unreachable (databook-postgres not running)");
        return;
    }

    let home = scratch_home("mem-on-refresh");
    let _cleanup = HomeGuard(home.clone());
    let config = write_docker_config(&home);

    // Seed the store with a real schema cache write. The refresh needs no
    // model — only the DB — so it runs with the offline smoke config. If it
    // fails (container up but pagila missing, wrong password, …) skip rather
    // than assert, per the packet.
    let refresh = run_saya_schema_refresh(&home, &config);
    if !refresh.success {
        eprintln!(
            "skipping rung 3: schema refresh failed (exit {:?}); stderr:\n{}",
            refresh.exit_code, refresh.stderr
        );
        return;
    }

    let screen = match paint_with_config(&home, &config.config, &config.connections) {
        Ok(screen) => screen,
        Err(reason) => {
            eprintln!("skipping rung 3 (pty): {reason}");
            return;
        }
    };
    assert_splash_for_profile(&screen, "docker_postgres", "rung 3 (after schema refresh)");
}
