use super::LocalStateEffect;

/// The back-compat guarantee: a `ToolEffect` serialized by an older build
/// (no `local_state` key) deserializes to the default `None`.
#[test]
fn tool_effect_without_local_state_key_defaults_to_none() {
    let json = r#"{
            "database_data": false,
            "external_side_effect": false,
            "requires_approval": false
        }"#;
    let effect: super::ToolEffect = serde_json::from_str(json).expect("old form deserializes");
    assert_eq!(effect.local_state, LocalStateEffect::None);
}

/// Each variant round-trips through snake_case.
#[test]
fn local_state_effect_round_trips_through_snake_case() {
    for (variant, expected) in [
        (LocalStateEffect::None, "none"),
        (LocalStateEffect::Read, "read"),
        (LocalStateEffect::WriteCandidate, "write_candidate"),
        (LocalStateEffect::WriteWorkspace, "write_workspace"),
        (LocalStateEffect::WriteSession, "write_session"),
    ] {
        let text = serde_json::to_string(&variant).expect("serializes");
        assert_eq!(text, format!("\"{expected}\""), "{variant:?}");
        let back: LocalStateEffect = serde_json::from_str(&text).expect("deserializes back");
        assert_eq!(back, variant, "{variant:?}");
    }
}
