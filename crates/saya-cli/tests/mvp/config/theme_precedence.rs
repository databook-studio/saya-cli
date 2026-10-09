/// An empty root has no config files and no flags: every explained setting,
/// `ui.theme` included, is a built-in default and no line names a config file
/// or the command line.
#[test]
fn config_show_explain_in_an_empty_root_names_the_default_layer() {
    let root = test_root("explain-empty-default");
    let output = saya_isolated(&root, &["--format", "text", "config", "show", "--explain"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ai.model"), "{stdout}");
    for line in stdout.lines() {
        assert!(
            line.ends_with("← built-in default"),
            "unexpected layer in an empty root: {line}"
        );
        assert!(
            !line.contains("config file") && !line.contains("command line"),
            "no file or command-line layer can win in an empty root: {line}"
        );
    }
    let theme_line = stdout
        .lines()
        .find(|line| line.starts_with("ui.theme"))
        .unwrap_or_else(|| panic!("no ui.theme line: {stdout}"));
    assert!(
        theme_line.ends_with("auto  ← built-in default"),
        "no flag and no config leaves the built-in default theme: {theme_line}"
    );
    let _ = fs::remove_dir_all(root);
}

/// Precedence matrix (a): `[ui] theme = 'dark'` in the user config wins when
/// no `--theme` flag is passed, and the provenance names the user config
/// file that supplied it.
#[test]
fn config_show_explain_user_config_theme_wins_without_a_flag() {
    let root = test_root("explain-theme-config-wins");
    fs::create_dir_all(root.join("user-config/saya")).unwrap();
    fs::write(
        root.join("user-config/saya/config.toml"),
        "[ui]\ntheme = 'dark'\n",
    )
    .unwrap();
    let output = saya_isolated(&root, &["config", "show", "--explain"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let theme_line = stdout
        .lines()
        .find(|line| line.starts_with("ui.theme"))
        .unwrap_or_else(|| panic!("no ui.theme line: {stdout}"));
    let expected = format!(
        "{:<38} {}  ← user config file ({})",
        "ui.theme",
        "dark",
        root.join("user-config/saya/config.toml").display()
    );
    assert_eq!(theme_line, expected, "{stdout}");
    let _ = fs::remove_dir_all(root);
}

/// Precedence matrix (b): `--theme auto` is an explicit override, not an
/// absence marker — it beats the config's theme and names the flag.
#[test]
fn config_show_explain_explicit_auto_overrides_the_config_theme() {
    let root = test_root("explain-theme-auto-override");
    fs::create_dir_all(root.join("user-config/saya")).unwrap();
    fs::write(
        root.join("user-config/saya/config.toml"),
        "[ui]\ntheme = 'dark'\n",
    )
    .unwrap();
    let output = saya_isolated(&root, &["--theme", "auto", "config", "show", "--explain"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let theme_line = stdout
        .lines()
        .find(|line| line.starts_with("ui.theme"))
        .unwrap_or_else(|| panic!("no ui.theme line: {stdout}"));
    assert!(
        theme_line.ends_with("auto  ← command line (--theme)"),
        "an explicit --theme auto must override the config: {theme_line}"
    );
    let _ = fs::remove_dir_all(root);
}

/// Precedence matrix (c): `--theme light` overrides the config's theme and
/// names the flag.
#[test]
fn config_show_explain_explicit_light_overrides_the_config_theme() {
    let root = test_root("explain-theme-light-override");
    fs::create_dir_all(root.join("user-config/saya")).unwrap();
    fs::write(
        root.join("user-config/saya/config.toml"),
        "[ui]\ntheme = 'dark'\n",
    )
    .unwrap();
    let output = saya_isolated(&root, &["--theme", "light", "config", "show", "--explain"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let theme_line = stdout
        .lines()
        .find(|line| line.starts_with("ui.theme"))
        .unwrap_or_else(|| panic!("no ui.theme line: {stdout}"));
    assert!(
        theme_line.ends_with("light  ← command line (--theme)"),
        "an explicit --theme light must override the config: {theme_line}"
    );
    let _ = fs::remove_dir_all(root);
}

/// `--no-color` reaches the real configuration boundary: the `output.color`
/// line in `config show --explain` names the command line, not the built-in
/// default the post-resolution mutation used to leave behind.
#[test]
fn config_show_explain_no_color_names_the_command_line_layer() {
    let root = test_root("explain-no-color-flag");
    let output = saya_isolated(&root, &["--no-color", "config", "show", "--explain"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let color_line = stdout
        .lines()
        .find(|line| line.starts_with("output.color"))
        .unwrap_or_else(|| panic!("no output.color line: {stdout}"));
    assert!(
        color_line.ends_with("never  ← command line (--no-color)"),
        "the flag must resolve and name its source: {color_line}"
    );
    let _ = fs::remove_dir_all(root);
}

/// Colour precedence through the real boundary: without the flag the built-in
/// default holds; a user-config `[output] color` wins with its file
/// attribution; `--no-color` over that config still wins and names the flag.
#[test]
fn config_show_explain_output_color_precedence_matches_the_other_layers() {
    let root = test_root("explain-color-default");
    let output = saya_isolated(&root, &["config", "show", "--explain"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let color_line = stdout
        .lines()
        .find(|line| line.starts_with("output.color"))
        .unwrap_or_else(|| panic!("no output.color line: {stdout}"));
    assert!(
        color_line.ends_with("auto  ← built-in default"),
        "no flag and no config leaves the built-in default: {color_line}"
    );
    let _ = fs::remove_dir_all(root);

    let root = test_root("explain-color-config-wins");
    fs::create_dir_all(root.join("user-config/saya")).unwrap();
    fs::write(
        root.join("user-config/saya/config.toml"),
        "[output]\ncolor = 'always'\n",
    )
    .unwrap();
    let output = saya_isolated(&root, &["config", "show", "--explain"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let color_line = stdout
        .lines()
        .find(|line| line.starts_with("output.color"))
        .unwrap_or_else(|| panic!("no output.color line: {stdout}"));
    let expected = format!(
        "{:<38} {}  ← user config file ({})",
        "output.color",
        "always",
        root.join("user-config/saya/config.toml").display()
    );
    assert_eq!(color_line, expected, "{stdout}");

    let output = saya_isolated(&root, &["--no-color", "config", "show", "--explain"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let color_line = stdout
        .lines()
        .find(|line| line.starts_with("output.color"))
        .unwrap_or_else(|| panic!("no output.color line: {stdout}"));
    assert!(
        color_line.ends_with("never  ← command line (--no-color)"),
        "the flag overrides the config and names its source: {color_line}"
    );
    let _ = fs::remove_dir_all(root);
}
