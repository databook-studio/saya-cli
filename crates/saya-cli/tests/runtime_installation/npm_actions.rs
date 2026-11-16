use super::{temp_root, tree_paths};
use std::{env, fs, process::Command};

#[test]
fn npm_dependency_and_check_actions_require_yes_before_config_or_state() {
    let root = temp_root();
    let launch = root.0.join("launch");
    let project = root.0.join("project");
    fs::create_dir(&project).unwrap();
    let config = root.0.join("config-home/saya/config.toml");
    let invalid_config = b"not = [valid TOML";
    fs::write(&config, invalid_config).unwrap();
    let state_db = root.0.join("state.sqlite3");
    let before = tree_paths(&root.0);
    let actions = [
        vec![
            "--non-interactive".to_owned(),
            "runtime".to_owned(),
            "install-deps".to_owned(),
            "--project".to_owned(),
            project.display().to_string(),
        ],
        vec![
            "--non-interactive".to_owned(),
            "runtime".to_owned(),
            "check-node".to_owned(),
            "--project".to_owned(),
            project.display().to_string(),
            "--action".to_owned(),
            "test-ci".to_owned(),
        ],
    ];

    for args in actions {
        let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
        command
            .args(args)
            .current_dir(&launch)
            .env_clear()
            .env("HOME", root.0.join("home"))
            .env("SAYA_CONFIG_HOME", root.0.join("config-home"))
            .env("SAYA_STATE_DB", &state_db)
            .env("PATH", "")
            .env("LANG", "C")
            .env("LC_ALL", "C");
        for (name, value) in
            env::vars_os().filter(|(name, _)| name.to_string_lossy().starts_with("SAYA_TEST_"))
        {
            command.env(name, value);
        }
        let output = command.output().unwrap();
        let message = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        assert_eq!(
            output.status.code(),
            Some(2),
            "an npm action without explicit consent is refused: {message}"
        );
        assert!(
            message.contains("--yes"),
            "the refusal identifies the missing explicit consent: {message}"
        );
        assert_eq!(fs::read(&config).unwrap(), invalid_config);
        assert!(!state_db.exists());
        assert_eq!(tree_paths(&root.0), before);
    }
}
