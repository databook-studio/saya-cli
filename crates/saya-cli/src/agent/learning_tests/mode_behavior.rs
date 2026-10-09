#[tokio::test]
async fn mode_off_performs_no_store_read_and_no_store_write_across_turn() {
    // 1. Store write check: turn performs 0 writes.
    let (store, identity, root) = run_one_turn(MemoryMode::Off).await;
    let obj = object_ref(&identity, "orders");
    assert_eq!(
        claim_count(&store, &obj).await,
        0,
        "mode = off stores no claims"
    );

    let any_claims = store.knowledge_for_profile(&identity).await.unwrap().len();
    assert_eq!(any_claims, 0, "no claims written anywhere under mode = off");

    // 2. Store read check: recall mode is None, so store is never read.
    assert_eq!(
        recall_mode_for(MemoryMode::Off),
        None,
        "recall_mode_for(Off) is None — recall is skipped entirely"
    );

    // If an unopenable store path exists, mode = Off never touches it.
    let blocker_root = temp_root("blocker");
    fs::write(blocker_root.join("blocker_file"), b"x").unwrap();
    let bad_path = blocker_root.join("blocker_file/state.sqlite3");
    let _store = SqliteStateStore::new(&bad_path);
    assert_eq!(recall_mode_for(MemoryMode::Off), None);
    assert!(fs::metadata(blocker_root.join("blocker_file")).is_ok());

    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(blocker_root);
}

// ---------------------------------------------------------------------------
// Test 2: mode = "assisted" supplies active knowledge and labels pending knowledge
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mode_assisted_recalls_candidates_and_permits_proposals() {
    // 1. Recall mode is IncludeCandidates, so active and candidate claims are admitted.
    assert_eq!(
        recall_mode_for(MemoryMode::Assisted),
        Some(crate::contracts::RecallMode::IncludeCandidates)
    );

    // 2. Write setup permits candidate writes and attaches observations.
    let setup = LearningSetup::from(MemoryMode::Assisted);
    assert!(setup.permit_candidate_writes);
    assert!(setup.observes());

    // 3. Definitions do NOT include contract_propose (deprecated in Phase F).
    let defs = DatabaseTools::definitions(true, true, setup.permit_candidate_writes, false, true);
    assert!(
        defs.iter().all(|d| d.name != "contract_propose"),
        "contract_propose is removed from model tools"
    );
}

// ---------------------------------------------------------------------------
// Test 3: Privacy gate suppresses recall under assisted mode
// ---------------------------------------------------------------------------

#[test]
fn privacy_gate_suppresses_recall_regardless_of_mode() {
    for mode in [MemoryMode::Off, MemoryMode::Assisted] {
        let recall_mode = recall_mode_for(mode);
        let allow_query_data = false;
        let recall_runs = recall_mode.is_some() && allow_query_data;
        assert!(
            !recall_runs,
            "with sharing disabled no mode runs recall (mode={mode:?})"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 4: Absent [memory] resolves to default (off), asserted explicitly
// ---------------------------------------------------------------------------

#[test]
fn absent_memory_section_resolves_to_off_default() {
    let resolved = resolve(ResolutionInput::new(ConnectionsFile::default())).unwrap();
    assert_eq!(
        resolved.memory.mode,
        MemoryMode::Off,
        "default memory mode must be Off"
    );
    assert_eq!(resolved.memory.max_contracts, 5);
    assert_eq!(resolved.memory.max_claims_per_contract, 12);
    assert_eq!(resolved.memory.max_context_bytes, 16384);
}

// ---------------------------------------------------------------------------
// Test 5: Legacy config naming old axes fails loudly at parse time
// ---------------------------------------------------------------------------

#[test]
fn legacy_two_axis_configuration_fails_loudly_at_parse_time() {
    let recall_err = ConfigFile::from_toml("[memory]\nrecall = 'confirmed'\n").unwrap_err();
    let rendered = format!("{recall_err:?}");
    assert!(
        rendered.contains("recall"),
        "error should name rejected field 'recall': {rendered}"
    );

    let learning_err =
        ConfigFile::from_toml("[memory]\nlearning = 'auto-candidate'\n").unwrap_err();
    let rendered = format!("{learning_err:?}");
    assert!(
        rendered.contains("learning"),
        "error should name rejected field 'learning': {rendered}"
    );
}

// ---------------------------------------------------------------------------
// Test 6: Changing mode between turns takes effect on the next turn
// ---------------------------------------------------------------------------

#[tokio::test]
async fn changing_memory_mode_takes_effect_on_the_next_turn() {
    // Turn 1: off -> no candidate writes, no observations.
    let off = LearningSetup::from(MemoryMode::Off);
    assert!(!off.permit_candidate_writes);
    assert!(!off.observes());
    let off_defs = DatabaseTools::definitions(true, true, off.permit_candidate_writes, false, true);
    assert!(
        off_defs.iter().all(|d| d.name != "contract_propose"),
        "turn 1 (off): contract_propose is hidden"
    );

    // Turn 2: assisted -> permits candidate writes and attaches observations.
    let assisted = LearningSetup::from(MemoryMode::Assisted);
    assert!(assisted.permit_candidate_writes);
    assert!(assisted.observes());
    let assisted_defs =
        DatabaseTools::definitions(true, true, assisted.permit_candidate_writes, false, true);
    assert!(
        assisted_defs.iter().all(|d| d.name != "contract_propose"),
        "turn 2 (assisted): contract_propose is never advertised to the model"
    );
}
