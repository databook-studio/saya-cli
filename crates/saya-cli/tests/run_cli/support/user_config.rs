/// Writes the user-layer config the binary reads:
/// `<SAYA_CONFIG_HOME>/saya/config.toml`.
fn user_config(env: &TestEnv, toml: &str) {
    let dir = env.root.join("user-config").join("saya");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("config.toml"), toml).unwrap();
}
