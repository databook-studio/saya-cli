use super::*;

/// A compacted parent rotates into a successor that carries the summary,
/// notes, and facts plus the not-yet-summarised tail — and nothing else: no
/// usage, observations, denials, or prior outcomes ride across, and the
/// parent is never mutated.
#[test]
fn a_compacted_parent_rotates_into_a_small_successor() {
    let parent = compacted_parent();
    let snapshot = parent.clone();
    let record = current_record(&parent);
    assert_eq!(
        parent.compacted_turns, 4,
        "fixture sanity: 4 of 6 compacted"
    );
    let successor = rotate_state(&parent, "s2", "rot-1").expect("a compacted parent rotates");
    assert_eq!(parent, snapshot, "the parent is never mutated");

    assert_eq!(successor.id, "s2");
    assert_eq!(
        successor.parent,
        Some(ParentLink {
            id: "s1".into(),
            rotation_id: "rot-1".into(),
        }),
        "the link is lineage only"
    );
    assert_eq!(
        successor.turns,
        parent.turns[parent.compacted_turns..].to_vec(),
        "the verbatim not-yet-summarised tail rides across"
    );
    assert_eq!(successor.turns.len(), 2, "fixture sanity: the 2 kept turns");
    assert_eq!(successor.turns[0].user, "q4");
    assert_eq!(successor.compaction_summary.as_deref(), Some(SUMMARY));
    assert_eq!(successor.compacted_turns, 0);
    assert!(
        successor.messages.is_empty(),
        "the transcript does not ride"
    );
    assert!(
        successor.prior_tool_outcomes.is_none(),
        "a successor inherits no live outcome status"
    );
    assert_eq!(
        successor.observations,
        Default::default(),
        "a successor inherits no observations"
    );
    assert!(
        successor.denied_programs.is_empty(),
        "a successor inherits NO denials"
    );
    assert_eq!(
        successor.usage,
        SessionUsage::default(),
        "a successor starts with a fresh token total"
    );
    assert!(!successor.context_warned);
    assert!(!successor.auto_compact_failed);

    let Some(StoredContinuation::Current(carried)) = successor.continuation.as_ref() else {
        panic!("the successor carries a typed record");
    };
    carried
        .validate()
        .expect("the carried record passes the contract");
    assert_eq!(
        carried.source.through_turn, 0,
        "it summarises none of these turns"
    );
    assert_eq!(
        carried.source.source_sha256,
        source_digest(&[]),
        "the boundary digests the empty prefix"
    );
    assert_eq!(
        carried.source.workspace_root.as_deref(),
        Some("/projects/saya"),
        "the boundary carries the workspace"
    );
    assert_eq!(
        carried.source.profile.as_deref(),
        Some("analytics"),
        "the boundary carries the profile"
    );
    assert_eq!(
        carried.narrative, record.narrative,
        "the narrative is carried"
    );
    assert_eq!(carried.user_notes, record.user_notes, "notes ride across");
    assert_eq!(carried.files, record.files, "file facts ride across");
    assert_eq!(carried.checks, record.checks, "check facts ride across");

    assert_eq!(successor.profile.as_deref(), Some("analytics"));
    assert_eq!(successor.workspace_root.as_deref(), Some("/projects/saya"));
    assert_eq!(successor.approval_mode, parent.approval_mode);
    assert_eq!(successor.agent_mode, parent.agent_mode);
    assert_eq!(successor.provider, parent.provider);
    assert_eq!(successor.model, parent.model);
    assert_eq!(successor.allow_data_sharing, parent.allow_data_sharing);
    assert_eq!(successor.included_profiles, parent.included_profiles);
    assert_eq!(successor.task_list, parent.task_list);
    assert_eq!(successor.provider_endpoint, parent.provider_endpoint);
    assert_eq!(
        successor.provider_endpoint_bound,
        parent.provider_endpoint_bound
    );
}

/// A successor built from a large parent serializes strictly smaller than
/// the parent: the whole point of rotation is the small record.
#[test]
fn a_successor_from_a_large_parent_is_strictly_smaller_than_its_parent() {
    let root = store_root("size");
    let store = FsSessionStore::new(&root);
    let mut state = turns(200);
    compact(&mut state, SUMMARY);
    let parent_bytes = store
        .capacity_of(state.redacted())
        .expect("the parent measures")
        .used_bytes;
    let successor = rotate_state(&state, "s2", "rot-big").expect("a compacted parent rotates");
    let successor_bytes = store
        .capacity_of(successor.redacted())
        .expect("the successor measures")
        .used_bytes;
    assert!(
        successor_bytes < parent_bytes,
        "the successor ({successor_bytes}B) must be smaller than the parent ({parent_bytes}B)"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// The published successor saves through the real store and its carried
/// summary is restored on resume: summary present, `compacted_turns` zero,
/// lineage persisted.
#[test]
fn a_published_successor_resumes_with_its_carried_summary() {
    let root = store_root("resume");
    let parent = compacted_parent();
    let successor = rotate_state(&parent, "s2", "rot-1").expect("a compacted parent rotates");
    let store = FsSessionStore::new(&root);
    block_on(store.save(successor.redacted())).expect("the successor saves");
    let loaded = block_on(store.load("s2"))
        .expect("loads")
        .expect("the record is there");
    let resumed = state_from_redacted(loaded, &defaults());
    assert_eq!(
        resumed.compaction_summary.as_deref(),
        Some(SUMMARY),
        "the carried narrative is restored as the live summary"
    );
    assert_eq!(resumed.compacted_turns, 0);
    assert_eq!(
        resumed.parent,
        Some(ParentLink {
            id: "s1".into(),
            rotation_id: "rot-1".into(),
        }),
        "the lineage link persists"
    );
    let _ = std::fs::remove_dir_all(root);
}
