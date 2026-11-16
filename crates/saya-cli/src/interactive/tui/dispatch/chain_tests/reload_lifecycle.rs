/// `/reload` through the real dispatch: the ONE new outcome, `Dispatch::Reload`
/// — the dispatch never reloads itself (the loop's `tick_pending` owns the
/// swap under the busy gate), it only routes the request.
#[test]
fn reload_dispatch_returns_the_reload_outcome() {
    let mut fx = ChainFixture::build("reload-dispatch");
    assert!(
        matches!(dispatch_line("/reload", &mut fx), Dispatch::Reload),
        "the real dispatch of /reload must route the swap"
    );
}

/// `/reload now` is a parse error, not an argument: an Error block with the
/// pinned wording, and a Handled outcome.
#[test]
fn reload_with_trailing_text_is_a_parse_error_block() {
    let mut fx = ChainFixture::build("reload-parse-error");
    assert!(
        matches!(dispatch_line("/reload now", &mut fx), Dispatch::Handled),
        "the parse error is inline"
    );
    let error =
        last_block(&fx.app.transcript, BlockKind::Error).expect("a parse error is an error block");
    assert_eq!(
        error, "reload takes no argument: /reload",
        "the pinned parse-error wording: {error}"
    );
}

/// `/reload` drained through the real `tick_pending`, over a fixture whose
/// launch options point `--config` at a fixture file: unchanged files say so;
/// an edited file swaps `app.runtime` and names the key; a broken file leaves
/// an Error block and the OLD runtime handle in place (`Arc::ptr_eq`).
#[test]
fn reload_drained_through_tick_pending_swaps_on_success_and_keeps_the_old_on_failure() {
    use crate::config::reload::{ReloadInputs, build_candidate};
    use std::sync::Arc;
    let mut fx = ChainFixture::build("reload-tick");
    let config = fx.root.0.join("config.toml");
    std::fs::write(
        &config,
        "[ai]\nmodel = \"test-model\"\n[run]\nmax_rows = 100\n",
    )
    .unwrap();
    // The launch options the loop carries: the same `--config` the app's
    // runtime was built from, so a reload re-reads exactly that file. The
    // seed candidate is built through the SAME inputs production reloads
    // with (`from_launch`: real env, cwd "."), so an unchanged file is
    // genuinely unchanged and the edit is the only delta.
    fx.launch_options = crate::cli::GlobalOptions {
        config: Some(config.clone()),
        ..Default::default()
    };
    let seed_inputs = ReloadInputs::from_launch(&fx.launch_options);
    let old = build_candidate(&seed_inputs).expect("the seed candidate builds");
    fx.app.runtime = Arc::new(old);

    // Unchanged: the System block is the unchanged line.
    fx.app.pending = Some("/reload".to_string());
    drain_through_tick_pending(&mut fx);
    assert!(fx.app.pending.is_none(), "the reload consumed the line");
    let said =
        last_block(&fx.app.transcript, BlockKind::System).expect("the unchanged reload is said");
    assert_eq!(said, "Configuration is unchanged.");

    // Edited: the swap is real (a new value under the same Arc slot the next
    // tick derives its handle from) and the block names the key.
    std::fs::write(
        &config,
        "[ai]\nmodel = \"test-model\"\n[run]\nmax_rows = 7\n",
    )
    .unwrap();
    fx.app.pending = Some("/reload".to_string());
    drain_through_tick_pending(&mut fx);
    assert_eq!(
        fx.app.runtime.resolved.max_rows, 7,
        "the app's runtime handle carries the new value"
    );
    let named =
        last_block(&fx.app.transcript, BlockKind::System).expect("the edited reload is said");
    assert!(
        named.contains("max_rows") && named.contains("100 -> 7"),
        "the plan names the changed key and values: {named}"
    );

    // Broken: an Error block says the previous configuration is still in
    // use, and the handle IS the old one — same allocation, not a fresh
    // Arc over equal contents.
    let before = Arc::clone(&fx.app.runtime);
    std::fs::write(&config, "[run\n").unwrap();
    fx.app.pending = Some("/reload".to_string());
    drain_through_tick_pending(&mut fx);
    let error = last_block(&fx.app.transcript, BlockKind::Error).expect("the failure is said");
    assert!(
        error.starts_with("Reload failed: ")
            && error.contains("The previous configuration is still in use."),
        "the pinned failure sentence: {error}"
    );
    assert!(
        Arc::ptr_eq(&fx.app.runtime, &before),
        "a failed reload keeps the old runtime handle"
    );
}

/// `/reload` drained through the real `tick_pending` retires a pending
/// clarification exactly when the reload's re-seed moved the answering
/// context: the key is captured before the re-seed and the shared abandon
/// step runs after a successful reload. A reseeded model (an untouched
/// session) moves the key; a live-only key (max_rows) does not.
#[test]
fn a_reload_that_moves_the_context_key_abandons_the_pending_question() {
    let mut fx = ChainFixture::build("reload-model");
    seed_runtime_from_launch_options(&mut fx);
    // The untouched session: its model copy IS the runtime's, so a changed
    // config model is re-seeded onto it.
    fx.state.model = "test-model".to_string();
    fx.session.abandon_question();
    let pending = fx
        .session
        .begin_question(&fx.runtime)
        .expect("the question begins");
    fx.session.settle_question(&pending, true);

    let config = fx.launch_options.config.clone().expect("the --config path");
    std::fs::write(&config, "[ai]\nmodel = \"m2\"\n[run]\nmax_rows = 100\n")
        .expect("the model edit writes");
    fx.app.pending = Some("/reload".to_string());
    drain_through_tick_pending(&mut fx);
    assert_eq!(
        fx.state.model, "m2",
        "the reload re-seeded the untouched model copy"
    );
    assert!(
        pending.cancellation().is_cancelled(),
        "the reseeded model moved the answering context: the pending question is abandoned"
    );
    let next = fx
        .session
        .begin_question(&fx.runtime)
        .expect("the next question begins");
    assert!(
        !next.is_same_question(&pending),
        "the next begin mints a different question after the reload"
    );
}

/// The preserve side of the same rule: a reload whose only change is a LIVE
/// key (max_rows) re-seeds nothing, so the key stands and the pending
/// clarification survives with its token live.
#[test]
fn a_reload_that_changes_only_a_live_key_preserves_the_pending_question() {
    let mut fx = ChainFixture::build("reload-rows");
    seed_runtime_from_launch_options(&mut fx);
    fx.state.model = "test-model".to_string();
    fx.session.abandon_question();
    let pending = fx
        .session
        .begin_question(&fx.runtime)
        .expect("the question begins");
    fx.session.settle_question(&pending, true);

    let config = fx.launch_options.config.clone().expect("the --config path");
    std::fs::write(
        &config,
        "[ai]\nmodel = \"test-model\"\n[run]\nmax_rows = 7\n",
    )
    .expect("the max_rows edit writes");
    fx.app.pending = Some("/reload".to_string());
    drain_through_tick_pending(&mut fx);
    assert_eq!(
        fx.state.model, "test-model",
        "no reseed fired for a live-only change"
    );
    let next = fx
        .session
        .begin_question(&fx.runtime)
        .expect("the next question begins");
    assert!(
        next.is_same_question(&pending),
        "a live-only reload preserves the pending question"
    );
    assert!(
        !next.cancellation().is_cancelled(),
        "the preserved question's token stays live"
    );
}

/// A `/reload` submitted while a request is running WAITS: the busy gate
/// holds the line in the queue like every command — it never overlaps a
/// stream, query, compaction, or replay — and once idle the queued line
/// drains and the swap happens.
#[test]
fn a_reload_submitted_while_busy_waits_until_idle() {
    use std::sync::Arc;
    let mut fx = ChainFixture::build("reload-busy");
    let config = fx.root.0.join("config.toml");
    std::fs::write(
        &config,
        "[ai]\nmodel = \"test-model\"\n[run]\nmax_rows = 100\n",
    )
    .unwrap();
    fx.launch_options = crate::cli::GlobalOptions {
        config: Some(config.clone()),
        ..Default::default()
    };
    let old = fx.app.runtime.clone();
    let (_tx, rx) = std::sync::mpsc::channel();
    fx.app.compact_task = Some(rx);
    fx.app.pending = Some("/reload".to_string());

    drain_through_tick_pending(&mut fx);
    assert!(
        fx.app.pending.is_some(),
        "while busy, the reload stays queued"
    );
    assert_eq!(
        fx.app.runtime.resolved.max_rows, 100,
        "nothing reloaded while the request runs"
    );

    fx.app.compact_task = None;
    fx.app.pending = Some("/reload".to_string());
    std::fs::write(
        &config,
        "[ai]\nmodel = \"test-model\"\n[run]\nmax_rows = 9\n",
    )
    .unwrap();
    drain_through_tick_pending(&mut fx);
    assert!(fx.app.pending.is_none(), "the queued reload was consumed");
    assert_eq!(
        fx.app.runtime.resolved.max_rows, 9,
        "the swap happened only once idle"
    );
    assert!(
        !Arc::ptr_eq(&fx.app.runtime, &old),
        "the swap is a new handle, never a mutation of the old Arc"
    );
}
