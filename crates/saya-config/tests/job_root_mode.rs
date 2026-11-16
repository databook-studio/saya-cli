//! `[job] root_mode`: the default, the two accepted values, and the protected
//! table's refusal of an untrusted project-layer statement.

use saya_config::{
    ConfigError, ConfigFile, ConnectionsFile, JobRootMode, ResolutionInput, resolve,
};

fn user_with(toml: &str) -> ResolutionInput {
    ResolutionInput::new(ConnectionsFile::default())
        .with_user(ConfigFile::from_toml(toml).expect("user fixture must parse"))
}

#[test]
fn default_is_in_place() {
    assert_eq!(JobRootMode::default(), JobRootMode::InPlace);
    let resolved = resolve(ResolutionInput::new(ConnectionsFile::default()))
        .expect("an empty config resolves");
    assert_eq!(resolved.coding.root_mode, JobRootMode::InPlace);
}

#[test]
fn managed_parses_and_resolves() {
    let file = ConfigFile::from_toml("[job]\nroot_mode = \"managed\"\n").expect("[job] parses");
    assert_eq!(file.job.root_mode, Some(JobRootMode::Managed));
    let resolved =
        resolve(user_with("[job]\nroot_mode = \"managed\"\n")).expect("a managed config resolves");
    assert_eq!(resolved.coding.root_mode, JobRootMode::Managed);
    let in_place = resolve(user_with("[job]\nroot_mode = \"in_place\"\n"))
        .expect("an in_place config resolves");
    assert_eq!(in_place.coding.root_mode, JobRootMode::InPlace);
}

#[test]
fn an_unknown_value_is_refused() {
    let error = ConfigFile::from_toml("[job]\nroot_mode = \"isolated\"\n")
        .expect_err("an unknown root mode must fail");
    assert!(
        matches!(error, ConfigError::Parse(_)),
        "an unknown value is a parse failure: {error}"
    );
    let message = error.to_string();
    assert!(
        message.contains("in_place") && message.contains("managed"),
        "the refusal names the two accepted values: {error}"
    );
}

#[test]
fn a_project_layer_cannot_set_it() {
    let input = ResolutionInput::new(ConnectionsFile::default()).with_project(
        ConfigFile::from_toml("[job]\nroot_mode = \"managed\"\n").expect("project fixture parses"),
    );
    let resolved = resolve(input).expect("resolution succeeds");
    assert_eq!(
        resolved.coding.root_mode,
        JobRootMode::InPlace,
        "the untrusted statement is reverted to the default"
    );
    assert!(
        resolved
            .ignored_project_overrides
            .contains(&"job".to_string()),
        "the report names the protected section: {:?}",
        resolved.ignored_project_overrides
    );
}
