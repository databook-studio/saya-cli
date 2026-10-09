#[test]
fn the_text_rendering_pins_one_aligned_line_per_setting() {
    let explanation = ConfigExplanation {
        settings: vec![
            SettingExplanation {
                key: "ai.model".into(),
                value: serde_json::json!("test-model"),
                source: "user config file".into(),
                layer: SettingSource::UserFile,
                file: Some("/u/c/config.toml".into()),
                reference: None,
            },
            SettingExplanation {
                key: "run.max_rows".into(),
                value: serde_json::json!(50),
                source: "built-in default".into(),
                layer: SettingSource::Default,
                file: None,
                reference: None,
            },
            SettingExplanation {
                key: "ai.api_key".into(),
                value: serde_json::json!("env:SAYA_AI_API_KEY"),
                source: "environment (SAYA_AI_API_KEY)".into(),
                layer: SettingSource::Env {
                    var: "SAYA_AI_API_KEY".into(),
                },
                file: None,
                reference: Some("env:SAYA_AI_API_KEY".into()),
            },
        ],
        ignored_project_overrides: vec![],
    };
    assert_eq!(
        text(&explanation),
        format!(
            "{:<38} {}  ← {}\n{:<38} {}  ← {}\n{:<38} {}  ← {}",
            "ai.model",
            "test-model",
            "user config file (/u/c/config.toml)",
            "run.max_rows",
            "50",
            "built-in default",
            "ai.api_key",
            "env:SAYA_AI_API_KEY",
            "environment (SAYA_AI_API_KEY)",
        )
    );
}

#[test]
fn the_text_rendering_lists_ignored_project_overrides_after_a_blank_line() {
    let explanation = ConfigExplanation {
        settings: vec![SettingExplanation {
            key: "ai.base_url".into(),
            value: serde_json::json!("https://user.example/v1"),
            source: "user config file".into(),
            layer: SettingSource::UserFile,
            file: Some("/u/c/config.toml".into()),
            reference: None,
        }],
        ignored_project_overrides: vec![IgnoredOverride {
            key: "ai.base_url".into(),
            reason: "the project file cannot set ai.base_url: it is security-critical; \
                     set it in your user config file or pass --trust-project-config"
                .into(),
        }],
    };
    assert_eq!(
        text(&explanation),
        format!(
            "{:<38} {}  ← {}\n\nIgnored from the project file:\n  {}",
            "ai.base_url",
            "https://user.example/v1",
            "user config file (/u/c/config.toml)",
            "the project file cannot set ai.base_url: it is security-critical; \
             set it in your user config file or pass --trust-project-config",
        )
    );
}
