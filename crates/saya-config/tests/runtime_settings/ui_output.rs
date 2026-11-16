use super::*;

/// The `[ui] theme` setting resolves from the config file, and a config that
/// sets nothing keeps `auto` — the default a user who never touches the
/// setting gets.
#[test]
fn ui_theme_resolves_from_file_with_auto_default() {
    let config = ConfigFile::from_toml("[ui]\ntheme = 'light'\n").unwrap();
    let resolved = saya_config::resolve(
        saya_config::ResolutionInput::new(ConnectionsFile::default()).with_user(config),
    )
    .unwrap();
    assert_eq!(resolved.ui_theme, ThemeChoice::Light);

    let defaults =
        saya_config::resolve(saya_config::ResolutionInput::new(ConnectionsFile::default()))
            .unwrap();
    assert_eq!(defaults.ui_theme, ThemeChoice::Auto);
}

/// The `--theme` CLI override has the highest precedence, so a flag wins over
/// a `[ui] theme` value the config file declared.
#[test]
fn cli_theme_flag_overrides_the_config_file_value() {
    let config = ConfigFile::from_toml("[ui]\ntheme = 'light'\n").unwrap();
    let resolved = saya_config::resolve(
        saya_config::ResolutionInput::new(ConnectionsFile::default())
            .with_user(config)
            .with_cli(saya_config::CliOverrides {
                theme: Some(ThemeChoice::Dark),
                ..Default::default()
            }),
    )
    .unwrap();
    assert_eq!(resolved.ui_theme, ThemeChoice::Dark);
}

/// The `--no-color` CLI override resolves through the ordinary layer chain:
/// the effective value becomes `Never` and the provenance names the flag, so
/// `config show --explain` attributes `output.color` to the command line
/// instead of the resolver's value being mutated after the fact.
#[test]
fn cli_no_color_flag_resolves_never_and_names_its_source() {
    let config = ConfigFile::from_toml("[output]\ncolor = 'always'\n").unwrap();
    let resolved = saya_config::resolve(
        saya_config::ResolutionInput::new(ConnectionsFile::default())
            .with_user(config)
            .with_cli(saya_config::CliOverrides {
                color: Some(ColorChoice::Never),
                ..Default::default()
            }),
    )
    .unwrap();
    assert_eq!(resolved.output_color, ColorChoice::Never);
    let origin = resolved.provenance.get("output.color").unwrap();
    assert_eq!(
        origin.source,
        SettingSource::Cli {
            flag: "--no-color".into()
        }
    );
}

/// Without the override the existing policy holds untouched: a config file's
/// `[output] color` wins with its file attribution, and with nothing anywhere
/// the built-in default `auto` holds with default provenance.
#[test]
fn without_the_color_override_the_existing_policy_holds() {
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(ConfigFile::from_toml("[output]\ncolor = 'always'\n").unwrap()),
    )
    .unwrap();
    assert_eq!(resolved.output_color, ColorChoice::Always);
    let origin = resolved.provenance.get("output.color").unwrap();
    assert_eq!(origin.source, SettingSource::UserFile);

    let resolved = resolve(ResolutionInput::new(ConnectionsFile::default())).unwrap();
    assert_eq!(resolved.output_color, ColorChoice::Auto);
    let origin = resolved.provenance.get("output.color").unwrap();
    assert_eq!(origin.source, SettingSource::Default);
}
