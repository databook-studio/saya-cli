#[test]
fn a_secret_reference_shows_only_its_redacted_label_and_never_a_value() {
    const SENTINEL: &str = "sentinel-explain-key-never-print";
    let env = BTreeMap::from([("SAYA_AI_API_KEY".to_string(), SENTINEL.to_string())]);
    let runtime = runtime_from(
        "[ai]\nmodel = 'm'\n",
        "",
        CliOverrides::default(),
        env,
        None,
    );
    let explanation = explain(&runtime);
    let api_key = entry(&explanation, "ai.api_key");
    assert_eq!(api_key.value, serde_json::json!("env:SAYA_AI_API_KEY"));
    assert_eq!(api_key.reference.as_deref(), Some("env:SAYA_AI_API_KEY"));
    let serialized = serde_json::to_string(&explanation).unwrap();
    assert!(!serialized.contains(SENTINEL), "{serialized}");
    let rendered = text(&explanation);
    assert!(!rendered.contains(SENTINEL), "{rendered}");
}

// --- the wire shape ---

#[test]
fn the_serialized_shape_is_settings_plus_ignored_project_overrides() {
    let runtime = runtime_from("", "", CliOverrides::default(), BTreeMap::new(), None);
    let explanation = explain(&runtime);
    let value: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&explanation).unwrap()).unwrap();
    let settings = value["settings"].as_array().unwrap();
    assert!(value["ignored_project_overrides"].is_array());
    assert_eq!(settings[0]["key"], "profile_name");
    assert_eq!(settings[0]["source"], "built-in default");
    assert_eq!(settings[0]["layer"]["layer"], "default");
    assert!(settings[0]["value"].is_string());
    assert!(settings[0].get("file").is_none(), "{:?}", settings[0]);
    assert!(settings[0].get("reference").is_none(), "{:?}", settings[0]);
}
