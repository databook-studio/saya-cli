use saya_config::{ConfigFile, ConnectionsFile, ResolutionInput, resolve};
const PROFILE: &str = r#"
[job.extensions.lookup]
protocol = "mcp"
program = "local-server"
argv = ["private-extension-argument"]
[job.extensions.lookup.tools]
lookup = "read_only"
mutate = "mutating"
"#;
const LSP_PROFILE: &str = r#"
[job]
automatic_lsp_profile = "rust"

[job.extensions.rust]
protocol = "lsp"
program = "local-server"
argv = []
language = "rust"
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

#[test]
fn automatic_lsp_selector_defaults_off_and_requires_a_known_lsp_profile() {
    let default = resolve(ResolutionInput::new(ConnectionsFile::default())).unwrap();
    assert_eq!(default.coding.automatic_lsp_profile, None);

    let selected = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml(LSP_PROFILE).unwrap()),
    )
    .unwrap();
    assert_eq!(
        selected.coding.automatic_lsp_profile.as_deref(),
        Some("rust")
    );
    let raw = ConfigFile::from_toml(LSP_PROFILE).unwrap();
    assert_eq!(
        raw.redacted_diagnostics()
            .job_automatic_lsp_profile
            .as_deref(),
        Some("rust")
    );
    assert_eq!(
        selected
            .redacted_diagnostics()
            .job_automatic_lsp_profile
            .as_deref(),
        Some("rust")
    );

    let unknown = ConfigFile::from_toml("[job]\nautomatic_lsp_profile = \"missing\"\n").unwrap();
    assert!(resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(unknown)).is_err());

    let mcp = ConfigFile::from_toml(&format!(
        "[job]\nautomatic_lsp_profile = \"lookup\"\n{PROFILE}"
    ))
    .unwrap();
    assert!(resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(mcp)).is_err());
}
