/// The shared swap helper (`apply_reloaded_runtime`) refreshes everything
/// the first screen and the dispatch read from the runtime: the profile
/// list, the splash's startup facts, and the @-references. Seeded with the
/// fixture's AI-ready ollama runtime and swapped onto an
/// openai-without-key runtime with a new profile, the AI line must flip
/// and the profile list must follow.
#[test]
fn apply_reloaded_runtime_refreshes_profiles_and_startup_from_the_new_runtime() {
    use crate::interactive::tui::loop_tick::pending::apply_reloaded_runtime;
    let mut fx = ChainFixture::build("apply-helper");
    assert!(
        fx.app.startup.ai.ready,
        "the fixture's ollama runtime reads ready: {:?}",
        fx.app.startup.ai
    );
    let mut new = unused_runtime();
    new.resolved.ai.provider = saya_config::AiProvider::Openai;
    new.connections.profiles.insert(
        "team".to_string(),
        saya_types::DatabaseProfile::Sqlite {
            path: fx.root.0.join("team.db").display().to_string(),
            read_only: true,
        },
    );
    apply_reloaded_runtime(&mut fx.app, &mut fx.state, Box::new(new));
    assert!(
        !fx.app.startup.ai.ready,
        "the openai-without-key runtime reads not ready: {:?}",
        fx.app.startup.ai
    );
    assert_eq!(fx.app.profiles, vec!["team".to_string()]);
    assert!(
        matches!(
            fx.app.runtime.resolved.ai.provider,
            saya_config::AiProvider::Openai
        ),
        "the app's runtime handle carries the new runtime"
    );
}

/// Seeds the fixture's runtime from the SAME inputs the reload reads, with
/// `--config` pointed at a fixture file — the O04 tick-test trick, so an
/// unwritten file is genuinely unchanged and an edit is the only delta.
fn seed_runtime_from_launch_options(fx: &mut ChainFixture) {
    use crate::config::reload::{ReloadInputs, build_candidate};
    use std::sync::Arc;
    let config = fx.root.0.join("config.toml");
    std::fs::write(
        &config,
        "[ai]\nmodel = \"test-model\"\n[run]\nmax_rows = 100\n",
    )
    .expect("the seed config writes");
    fx.launch_options = crate::cli::GlobalOptions {
        config: Some(config),
        ..Default::default()
    };
    let seed =
        build_candidate(&ReloadInputs::from_launch(&fx.launch_options)).expect("the seed builds");
    fx.app.runtime = Arc::new(seed);
}

/// The post-setup apply (U07): a closed wizard (`Finished(0)`) runs the
/// same reload `/reload` runs and renders ONE System block — the close
/// plus the reload report — and the swap is live.
#[test]
fn a_finished_setup_applies_the_reload_in_one_system_block() {
    use crate::interactive::tui::setup_apply::apply_setup_outcome;
    use crate::interactive::tui::terminal_handoff::SetupHandoff;
    let mut fx = ChainFixture::build("apply-finished");
    seed_runtime_from_launch_options(&mut fx);

    // The wizard "wrote" a change: max_rows drops to 7.
    let config = fx.launch_options.config.clone().expect("the --config path");
    std::fs::write(
        &config,
        "[ai]\nmodel = \"test-model\"\n[run]\nmax_rows = 7\n",
    )
    .expect("the wizard's write");
    apply_setup_outcome(
        Ok(SetupHandoff::Finished(0)),
        &mut fx.app,
        &mut fx.state,
        &fx.launch_options,
    )
    .expect("the apply renders");
    let said = last_block(&fx.app.transcript, BlockKind::System).expect("the close block is said");
    assert!(
        said.starts_with("Guided setup closed."),
        "the close leads the one block: {said}"
    );
    assert!(
        said.contains("max_rows") && said.contains("100 -> 7"),
        "the reload report follows: {said}"
    );
    assert_eq!(
        fx.app.runtime.resolved.max_rows, 7,
        "the swap is live for the next tick"
    );
}

/// A non-zero exit never reloads: the wizard's own refusal (exit 2) keeps
/// the U05 Error block and the runtime — even though a file changed
/// underneath.
#[test]
fn a_failed_setup_exit_never_reloads() {
    use crate::interactive::tui::setup_apply::apply_setup_outcome;
    use crate::interactive::tui::terminal_handoff::SetupHandoff;
    use std::sync::Arc;
    let mut fx = ChainFixture::build("apply-exit");
    seed_runtime_from_launch_options(&mut fx);
    let before = Arc::clone(&fx.app.runtime);
    let config = fx.launch_options.config.clone().expect("the --config path");
    std::fs::write(
        &config,
        "[ai]\nmodel = \"test-model\"\n[run]\nmax_rows = 9\n",
    )
    .expect("the changed config writes");

    apply_setup_outcome(
        Ok(SetupHandoff::Finished(2)),
        &mut fx.app,
        &mut fx.state,
        &fx.launch_options,
    )
    .expect("the apply renders");
    let error = last_block(&fx.app.transcript, BlockKind::Error).expect("the exit block");
    assert_eq!(
        error,
        "Guided setup ended with exit code 2. Nothing was applied that the wizard did not report."
    );
    assert_eq!(
        fx.app.runtime.resolved.max_rows, 100,
        "the changed file was never read into the session"
    );
    assert!(Arc::ptr_eq(&fx.app.runtime, &before), "no swap happened");
}

/// A wizard failure keeps the U05 Error block and never reloads.
#[test]
fn a_failed_wizard_never_reloads() {
    use crate::interactive::tui::setup_apply::apply_setup_outcome;
    use crate::interactive::tui::terminal_handoff::SetupHandoff;
    use std::sync::Arc;
    let mut fx = ChainFixture::build("apply-wizard-fail");
    seed_runtime_from_launch_options(&mut fx);
    let before = Arc::clone(&fx.app.runtime);
    let config = fx.launch_options.config.clone().expect("the --config path");
    std::fs::write(
        &config,
        "[ai]\nmodel = \"test-model\"\n[run]\nmax_rows = 9\n",
    )
    .expect("the changed config writes");

    apply_setup_outcome(
        Ok(SetupHandoff::Failed("x".into())),
        &mut fx.app,
        &mut fx.state,
        &fx.launch_options,
    )
    .expect("the apply renders");
    let error = last_block(&fx.app.transcript, BlockKind::Error).expect("the failure block");
    assert_eq!(error, "Guided setup failed: x");
    assert_eq!(fx.app.runtime.resolved.max_rows, 100, "no reload happened");
    assert!(Arc::ptr_eq(&fx.app.runtime, &before), "no swap happened");
}

/// A wizard that wrote files but whose reload fails: the close is said,
/// the failure names it, and the OLD runtime handle stays — the session
/// keeps the configuration it loaded.
#[test]
fn a_failed_reload_after_a_finished_setup_keeps_the_old_runtime() {
    use crate::interactive::tui::setup_apply::apply_setup_outcome;
    use crate::interactive::tui::terminal_handoff::SetupHandoff;
    use std::sync::Arc;
    let mut fx = ChainFixture::build("apply-reload-fail");
    seed_runtime_from_launch_options(&mut fx);
    let before = Arc::clone(&fx.app.runtime);
    let config = fx.launch_options.config.clone().expect("the --config path");
    std::fs::write(&config, "[run\n").expect("the broken config writes");

    apply_setup_outcome(
        Ok(SetupHandoff::Finished(0)),
        &mut fx.app,
        &mut fx.state,
        &fx.launch_options,
    )
    .expect("the apply renders");
    let error = last_block(&fx.app.transcript, BlockKind::Error).expect("the failure block");
    assert!(
        error.starts_with("Guided setup closed, but the new configuration could not be applied:")
            && error.contains("The previous configuration is still in use."),
        "the close plus the planner's failure text: {error}"
    );
    assert!(
        Arc::ptr_eq(&fx.app.runtime, &before),
        "the old runtime stays in place"
    );
}

/// The piped REPL never enters this flow: its `/setup` refusal (U05) names
/// the shell command and carries none of the first screen's entry choices —
/// a `/setup` pointer headless would be a dead end, and the choices are TUI
/// copy the headless loop never renders. The refusal's emitted output IS
/// this shared constant (pinned through the real per-turn entry by
/// `setup_turn_refuses_with_the_saya_setup_pointer_and_ends_errored`), so
/// the assertion rides it.
#[test]
fn the_headless_setup_refusal_never_prints_the_first_screen_choices() {
    use crate::interactive::session::commands::SETUP_TUI_ONLY_ERROR;
    use crate::interactive::tui::ui::surface::DATABASE_NONE;
    assert!(
        SETUP_TUI_ONLY_ERROR.contains("`saya setup`"),
        "the refusal points at the shell command: {SETUP_TUI_ONLY_ERROR}"
    );
    assert!(
        !SETUP_TUI_ONLY_ERROR.contains("/setup"),
        "the refusal never points at the in-session command: {SETUP_TUI_ONLY_ERROR}"
    );
    assert!(
        !SETUP_TUI_ONLY_ERROR.contains(DATABASE_NONE),
        "no first-screen copy leaked into the headless refusal: {DATABASE_NONE}"
    );
}
