/// The closed wording over an unchanged configuration: `Finished(0)` — which
/// includes the wizard's own cancelled-nothing-wrote exit — runs the same
/// reload `/reload` runs and renders ONE System block, the close plus the
/// reload's unchanged line.
#[test]
fn finished_zero_over_an_unchanged_config_says_closed_and_unchanged() {
    let mut fx = PendingFixture::build("apply-unchanged");
    // The launch options point `--config` at a fixture file and the app's
    // runtime is seeded from the same inputs the reload reads, so the
    // unwritten file is genuinely unchanged.
    let config = fx.root.join("config.toml");
    std::fs::write(&config, "[ai]\nmodel = 'test-model'\n").expect("the seed config writes");
    let options = GlobalOptions {
        config: Some(config),
        ..Default::default()
    };
    let inputs = crate::config::reload::ReloadInputs::from_launch(&options);
    let old = crate::config::reload::build_candidate(&inputs).expect("the seed candidate builds");
    fx.app.runtime = Arc::new(old);

    super::setup_apply::apply_setup_outcome(
        Ok(SetupHandoff::Finished(0)),
        &mut fx.app,
        &mut fx.state,
        &options,
    )
    .expect("renders");
    let blocks = fx.app.transcript.blocks();
    assert_eq!(blocks.len(), 1, "one block, not two: {:?}", blocks);
    assert_eq!(blocks[0].kind, BlockKind::System);
    assert_eq!(
        blocks[0].text,
        "Guided setup closed. Configuration is unchanged."
    );
}

/// A non-zero exit code is an Error block naming the code and what was NOT
/// applied beyond what the wizard itself reported — and no reload runs.
#[test]
fn finished_nonzero_pushes_the_exit_code_error_block() {
    let mut fx = PendingFixture::build("apply-exit-code");
    let options = GlobalOptions::default();
    super::setup_apply::apply_setup_outcome(
        Ok(SetupHandoff::Finished(2)),
        &mut fx.app,
        &mut fx.state,
        &options,
    )
    .expect("renders");
    let blocks = fx.app.transcript.blocks();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].kind, BlockKind::Error);
    assert_eq!(
        blocks[0].text,
        "Guided setup ended with exit code 2. Nothing was applied that the wizard did not report."
    );
}

/// A wizard failure surfaces its rendered error text as an Error block.
#[test]
fn failed_pushes_the_failure_error_block() {
    let mut fx = PendingFixture::build("apply-failed");
    let options = GlobalOptions::default();
    super::setup_apply::apply_setup_outcome(
        Ok(SetupHandoff::Failed("x".into())),
        &mut fx.app,
        &mut fx.state,
        &options,
    )
    .expect("renders");
    let blocks = fx.app.transcript.blocks();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].kind, BlockKind::Error);
    assert_eq!(blocks[0].text, "Guided setup failed: x");
}

/// A lost terminal (the reacquire failed) propagates and renders nothing:
/// the loop must stop, not keep drawing into a terminal it no longer owns.
#[test]
fn io_error_propagates_without_a_block() {
    let mut fx = PendingFixture::build("apply-io-error");
    let options = GlobalOptions::default();
    let error = super::setup_apply::apply_setup_outcome(
        Err(io::Error::other("reacquire failed")),
        &mut fx.app,
        &mut fx.state,
        &options,
    )
    .expect_err("a lost terminal is an error, not a rendered outcome");
    assert!(error.to_string().contains("reacquire failed"), "{error}");
    assert!(
        fx.app.transcript.blocks().is_empty(),
        "nothing renders on a lost terminal"
    );
}
