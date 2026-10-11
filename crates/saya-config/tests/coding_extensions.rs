use saya_config::{ConfigFile, ConnectionsFile, ResolutionInput, resolve};
const PROFILE: &str = r#"
[coding.extensions.lookup]
protocol = "mcp"
program = "local-server"
argv = ["private-extension-argument"]
[coding.extensions.lookup.tools]
lookup = "read_only"
mutate = "mutating"
"#;
#[test]
fn explicit_extension_profiles_are_bounded_data_and_hide_arguments() {
    let file = ConfigFile::from_toml(PROFILE).unwrap();
    assert!(!format!("{file:?}").contains("private-extension-argument"));
    let resolved =
        resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(file)).unwrap();
    assert_eq!(resolved.coding.extensions.len(), 1);
    assert_eq!(
        resolved.coding.extensions["lookup"].program(),
        "local-server"
    );
    assert!(!format!("{:?}", resolved.coding).contains("private-extension-argument"));
}
#[test]
fn untrusted_project_cannot_supply_executable_extension_profiles() {
    let project = ConfigFile::from_toml(PROFILE).unwrap();
    let resolved =
        resolve(ResolutionInput::new(ConnectionsFile::default()).with_project(project)).unwrap();
    assert!(resolved.coding.extensions.is_empty());
}
#[test]
fn extension_bounds_refuse_unsafe_program_and_oversized_argv() {
    for text in [
        PROFILE.replace("local-server", "../server"),
        PROFILE.replace("private-extension-argument", &"x".repeat(8193)),
    ] {
        let file = ConfigFile::from_toml(&text).unwrap();
        assert!(resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(file)).is_err());
    }
}
