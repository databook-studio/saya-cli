//! `[ai] app_attribution` — the OpenRouter app-attribution opt-out.
//!
//! Attribution is on by default, applies only to an `openrouter.ai` host, and
//! is visible in the effective-config view the provider is built from.

use saya_config::{ConfigFile, ConnectionsFile, ResolutionInput, resolve};

fn resolve_user(toml: &str) -> saya_config::ResolvedConfig {
    let config = ConfigFile::from_toml(toml).expect("the config parses");
    resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(config))
        .expect("the config resolves")
}

#[test]
fn attribution_is_on_by_default_and_can_be_turned_off() {
    let default = resolve_user("[ai]\nmodel = 'm'\n");
    assert!(default.ai.app_attribution);

    let off = resolve_user("[ai]\napp_attribution = false\n");
    assert!(!off.ai.app_attribution);

    let on = resolve_user("[ai]\napp_attribution = true\n");
    assert!(on.ai.app_attribution);
}

#[test]
fn attribution_follows_the_usual_layer_precedence() {
    // The environment wins over files.
    let file = ConfigFile::from_toml("[ai]\napp_attribution = true\n").unwrap();
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(file)
            .with_process_env([("SAYA_APP_ATTRIBUTION", "false")]),
    )
    .unwrap();
    assert!(!resolved.ai.app_attribution);

    // The project layer overrides the user layer, like every ordinary scalar.
    let user = ConfigFile::from_toml("[ai]\napp_attribution = true\n").unwrap();
    let project = ConfigFile::from_toml("[ai]\napp_attribution = false\n").unwrap();
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(user)
            .with_project(project),
    )
    .unwrap();
    assert!(!resolved.ai.app_attribution);
}

#[test]
fn attribution_is_visible_in_config_show() {
    let resolved = resolve_user("[ai]\napp_attribution = false\n");
    let rendered = serde_json::to_value(resolved.redacted_diagnostics()).unwrap();
    assert_eq!(rendered["app_attribution"], false);

    let default = resolve_user("[ai]\nmodel = 'm'\n");
    let rendered = serde_json::to_value(default.redacted_diagnostics()).unwrap();
    assert_eq!(rendered["app_attribution"], true);
}
