#[test]
fn bounds_from_config_copy_the_resolved_numbers() {
    let memory = ResolvedMemory {
        mode: MemoryMode::Assisted,
        max_contracts: 3,
        max_claims_per_contract: 7,
        max_context_bytes: 2048,
    };
    let bounds = bounds_from(&memory);
    assert_eq!(bounds.max_objects, 3);
    assert_eq!(bounds.max_claims_per_object, 7);
    assert_eq!(bounds.max_bytes, 2048);
}

#[test]
fn learning_setup_translation() {
    let off = LearningSetup::from(MemoryMode::Off);
    assert!(!off.permit_candidate_writes);
    assert!(off.observations.is_none());

    let assisted = LearningSetup::from(MemoryMode::Assisted);
    assert!(assisted.permit_candidate_writes);
    assert!(assisted.observations.is_some());
}

#[test]
fn default_memory_is_mode_off() {
    let m = default_memory();
    assert_eq!(m.mode, MemoryMode::Off);
}
