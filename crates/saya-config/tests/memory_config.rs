use saya_config::{
    ConfigError, ConfigFile, ConnectionsFile, MemoryLearning, MemoryRecall, ResolutionInput,
    resolve,
};

/// An empty config resolves to the safe defaults, so upgrading SAYA changes nothing
/// until the user opts in (ADR 0002, plan §10). This test is the guard against default drift.
#[test]
fn an_empty_config_upgrades_to_the_safe_memory_defaults() {
    let resolved = resolve(ResolutionInput::new(ConnectionsFile::default())).unwrap();
    assert_eq!(resolved.memory.recall, MemoryRecall::Confirmed);
    assert_eq!(resolved.memory.learning, MemoryLearning::Off);
    assert_eq!(resolved.memory.max_contracts, 5);
    assert_eq!(resolved.memory.max_claims_per_contract, 12);
    assert_eq!(resolved.memory.max_context_bytes, 16384);
}

/// Each valid string parses to its variant. Kebab spellings differ only for the
/// multi-word variants; the single-word ones are identical in either casing.
#[test]
fn each_valid_mode_string_parses_to_its_variant() {
    let recall = ConfigFile::from_toml("[memory]\nrecall = 'off'\nlearning = 'off'\n").unwrap();
    assert_eq!(recall.memory.recall, Some(MemoryRecall::Off));

    let confirmed = ConfigFile::from_toml("[memory]\nrecall = 'confirmed'\n").unwrap();
    assert_eq!(confirmed.memory.recall, Some(MemoryRecall::Confirmed));

    let candidates = ConfigFile::from_toml("[memory]\nrecall = 'include-candidates'\n").unwrap();
    assert_eq!(
        candidates.memory.recall,
        Some(MemoryRecall::IncludeCandidates)
    );

    let suggest = ConfigFile::from_toml("[memory]\nlearning = 'suggest'\n").unwrap();
    assert_eq!(suggest.memory.learning, Some(MemoryLearning::Suggest));

    let auto = ConfigFile::from_toml("[memory]\nlearning = 'auto-candidate'\n").unwrap();
    assert_eq!(auto.memory.learning, Some(MemoryLearning::AutoCandidate));
}

/// An out-of-range number is a typed error naming the field and the accepted range.
/// These values size untrusted work, so the upper bounds stop a typo becoming an
/// unbounded scan.
#[test]
fn out_of_range_numbers_are_typed_errors_naming_the_field_and_range() {
    let cases: &[(&str, &str)] = &[
        ("max_contracts", "0"),
        ("max_contracts", "51"),
        ("max_claims_per_contract", "0"),
        ("max_claims_per_contract", "101"),
        ("max_context_bytes", "1023"),
        ("max_context_bytes", "28673"),
    ];
    for (field, value) in cases {
        let toml = format!("[memory]\n{field} = {value}\n");
        let config = ConfigFile::from_toml(&toml).unwrap();
        let error = resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(config))
            .unwrap_err();
        let rendered = format!("{error:?}");
        assert!(
            rendered.contains(field),
            "missing field name {field:?} in {rendered}"
        );
        assert!(
            matches!(error, ConfigError::MemoryRange { field: f, .. } if f == *field),
            "expected MemoryRange for {field}={value}, got {rendered}"
        );
    }
}

/// `max_context_bytes` is clamped below the agent message budget: a setting above
/// the ceiling is a typed error naming both the configured value and the maximum,
/// not a silent clamp (spec test 5). A user who wrote the old 256 KiB should learn
/// it is impossible, not have it quietly become something else.
#[test]
fn max_context_bytes_above_the_ceiling_names_the_value_and_the_maximum() {
    // The pre-P1 ceiling was 256 KiB; a user carrying that config forward must be
    // told it is now refused, and why.
    let config = ConfigFile::from_toml("[memory]\nmax_context_bytes = 262144\n").unwrap();
    let error =
        resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(config)).unwrap_err();
    let rendered = format!("{error:?}");
    assert!(
        rendered.contains("262144"),
        "error must name the configured value: {rendered}"
    );
    assert!(
        rendered.contains("28672"),
        "error must name the maximum (the ceiling): {rendered}"
    );
    assert!(
        matches!(
            error,
            ConfigError::MemoryRange {
                field: "max_context_bytes",
                value: 262144,
                max: 28672,
                ..
            }
        ),
        "expected MemoryRange naming the configured value and the ceiling: {rendered}"
    );
}

/// `max_context_bytes` at the ceiling is accepted (spec test 6): the boundary is
/// inclusive, only values above it are refused.
#[test]
fn max_context_bytes_at_the_ceiling_is_accepted() {
    let config = ConfigFile::from_toml("[memory]\nmax_context_bytes = 28672\n").unwrap();
    let resolved = resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(config))
        .expect("the ceiling is accepted");
    assert_eq!(resolved.memory.max_context_bytes, 28672);
}

/// An unknown mode string is a typed error naming the accepted values. These are the
/// user's own config words, not untrusted content, so naming them is safe.
#[test]
fn an_unknown_mode_string_names_the_accepted_values() {
    let recall_err = ConfigFile::from_toml("[memory]\nrecall = 'sideways'\n").unwrap_err();
    let rendered = format!("{recall_err:?}");
    assert!(rendered.contains("off"));
    assert!(rendered.contains("confirmed"));
    assert!(rendered.contains("include-candidates"));

    let learning_err = ConfigFile::from_toml("[memory]\nlearning = 'eager'\n").unwrap_err();
    let rendered = format!("{learning_err:?}");
    assert!(rendered.contains("off"));
    assert!(rendered.contains("suggest"));
    assert!(rendered.contains("auto-candidate"));
}

/// A partial `[memory]` section fills unspecified fields from the defaults.
#[test]
fn a_partial_memory_section_fills_unspecified_fields_from_the_defaults() {
    let config = ConfigFile::from_toml("[memory]\nrecall = 'off'\nmax_contracts = 3\n").unwrap();
    let resolved =
        resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(config)).unwrap();
    assert_eq!(resolved.memory.recall, MemoryRecall::Off);
    assert_eq!(resolved.memory.max_contracts, 3);
    assert_eq!(resolved.memory.learning, MemoryLearning::Off);
    assert_eq!(resolved.memory.max_claims_per_contract, 12);
    assert_eq!(resolved.memory.max_context_bytes, 16384);
}

/// The resolved values appear in redacted diagnostics so `saya config show --resolved`
/// can answer "is learning on?" without the user reading TOML.
#[test]
fn resolved_memory_values_appear_in_redacted_diagnostics() {
    let config = ConfigFile::from_toml(
        "[memory]\nrecall = 'include-candidates'\nlearning = 'auto-candidate'\nmax_contracts = 7\n",
    )
    .unwrap();
    let resolved =
        resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(config.clone()))
            .unwrap();
    let rendered = serde_json::to_string(&resolved.redacted_diagnostics()).unwrap();
    assert!(rendered.contains("include-candidates"));
    assert!(rendered.contains("auto-candidate"));
    assert!(rendered.contains("\"memory_max_contracts\":7"));

    // The file-level view reports the configured modes too.
    let file_diagnostics = serde_json::to_string(&config.redacted_diagnostics()).unwrap();
    assert!(file_diagnostics.contains("include-candidates"));
    assert!(file_diagnostics.contains("auto-candidate"));
}

/// Round-trip: a config with every field set resolves to exactly those values.
#[test]
fn a_fully_set_memory_section_round_trips() {
    let config = ConfigFile::from_toml(
        "[memory]\n\
         recall = 'include-candidates'\n\
         learning = 'auto-candidate'\n\
         max_contracts = 50\n\
         max_claims_per_contract = 100\n\
         max_context_bytes = 28672\n",
    )
    .unwrap();
    let resolved =
        resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(config)).unwrap();
    assert_eq!(resolved.memory.recall, MemoryRecall::IncludeCandidates);
    assert_eq!(resolved.memory.learning, MemoryLearning::AutoCandidate);
    assert_eq!(resolved.memory.max_contracts, 50);
    assert_eq!(resolved.memory.max_claims_per_contract, 100);
    assert_eq!(resolved.memory.max_context_bytes, 28672);
}

/// The lower and upper bounds are inclusive at both ends — the boundary values
/// resolve, only the values just outside are rejected.
#[test]
fn the_numeric_bounds_are_inclusive_at_both_ends() {
    let config = ConfigFile::from_toml(
        "[memory]\n\
         max_contracts = 1\n\
         max_claims_per_contract = 1\n\
         max_context_bytes = 1024\n",
    )
    .unwrap();
    let resolved =
        resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(config)).unwrap();
    assert_eq!(resolved.memory.max_contracts, 1);
    assert_eq!(resolved.memory.max_claims_per_contract, 1);
    assert_eq!(resolved.memory.max_context_bytes, 1024);

    let config = ConfigFile::from_toml(
        "[memory]\n\
         max_contracts = 50\n\
         max_claims_per_contract = 100\n\
         max_context_bytes = 28672\n",
    )
    .unwrap();
    let resolved =
        resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(config)).unwrap();
    assert_eq!(resolved.memory.max_contracts, 50);
    assert_eq!(resolved.memory.max_claims_per_contract, 100);
    assert_eq!(resolved.memory.max_context_bytes, 28672);
}
