/// Writes the user-layer config the binary reads:
/// `<SAYA_CONFIG_HOME>/saya/config.toml`.
fn user_config(env: &TestEnv, toml: &str) {
    let dir = env.root.join("user-config").join("saya");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("config.toml"), toml).unwrap();
}

/// A short provider retry schedule for a run whose provider never answers.
/// A connection that comes up and dies is a retryable transport failure, so the
/// default nine-attempt schedule spends ~90 s per episode call before the
/// engine's own bounded step retry runs. A test that asserts the engine's step
/// retry says so with this schedule instead of waiting out the default.
fn fast_retries(env: &TestEnv) {
    user_config(env, "[ai]\nretry_delays_ms = [1, 1]\n");
}
