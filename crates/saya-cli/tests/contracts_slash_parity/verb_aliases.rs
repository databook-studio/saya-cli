// K3: the knowledge store is one command. `/contracts confirm`, `/contracts
// reject`, `/contracts queue`, `/contracts confirm-all` and their five
// one-release spellings parse to the **same** `ContractsCommand`, so the store
// effect and the output cannot diverge — only the deprecated spelling's note is
// new. Detection, not demonstration: the two spellings run over two
// identically-seeded stores and their outputs are compared byte for byte (ids
// substituted for a placeholder, since two stores mint their own ids).

/// Seeds one pending candidate on `table` and returns the claim id the queue
/// shows for it.
async fn seeded_candidate(
    store: &SqliteStateStore,
    runtime: &RuntimeConfig,
    table: &str,
) -> String {
    let identity = identity_for(runtime, "local");
    let profile = ProfileIdentity::parse(&identity).unwrap();
    let object = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        table,
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let payload = ClaimPayload::table_alias(table).unwrap();
    let slot = KnowledgeSlot::TableAlias;
    let binding = SchemaBinding::derive(&slot, &payload).expect("slot/payload agree");
    store
        .put_knowledge_item(saya_store::KnowledgeItemRequest {
            object,
            slot,
            value: payload,
            source: ClaimOrigin::AssistantInferred,
            state: KnowledgeState::Pending,
            schema_binding_json: serde_json::to_string(&binding).unwrap(),
            fingerprint: unobserved_fingerprint(),
        })
        .await
        .unwrap();
    let (_cmd, code, out, err) = run_slash("/contracts queue", runtime, store, RenderFormat::Text).await;
    assert_eq!(code, 0, "/contracts queue stderr: {err}");
    out.lines()
        .find_map(|line| line.starts_with("ki-").then(|| {
            line.split_whitespace().next().unwrap_or_default().to_string()
        }))
        .unwrap_or_else(|| panic!("the queue lists the seeded candidate: {out}"))
}

/// Parses a slash line to its request and runs it through the shared seam the
/// two surfaces run (`run_contracts_request`), capturing what a user would see.
/// Returns the translated operation, the note the spelling states, the exit
/// code, and the captured (stdout, stderr).
async fn run_slash_request(
    line: &str,
    runtime: &RuntimeConfig,
    store: &SqliteStateStore,
) -> (ContractsCommand, Option<&'static str>, i32, String, String) {
    let request = match parse_slash_command(line) {
        Ok(Some(SlashCommand::Contracts(request))) => request,
        other => panic!("expected SlashCommand::Contracts for {line:?}, got {other:?}"),
    };
    let command = request.command.clone();
    let note = request.note;
    capture_output_start();
    let code = run_contracts_request(request, runtime, RenderFormat::Text, store)
        .await
        .unwrap();
    let (out, err) = capture_output_take();
    (command, note, code, out, err)
}

/// One store's output with its own claim id replaced, so two stores' renderings
/// can be compared as bytes.
fn masked(output: &str, id: &str) -> String {
    output.replace(id, "<claim-id>")
}

/// `/contracts confirm <id>` equals `/confirm <id>`: one operation, one store
/// effect, one rendering — and the deprecated spelling states its note once.
#[tokio::test]
async fn contracts_confirm_equals_the_deprecated_confirm() {
    let root = temp_root("confirm_verb");
    let (runtime, _name) = runtime_at(&root);
    let canonical_store = store_at_name(&root, "canonical.sqlite3").await;
    let alias_store = store_at_name(&root, "alias.sqlite3").await;

    let canonical_id = seeded_candidate(&canonical_store, &runtime, "orders").await;
    let alias_id = seeded_candidate(&alias_store, &runtime, "orders").await;
    let canonical_prefix: String = canonical_id.chars().take(8).collect();
    let alias_prefix: String = alias_id.chars().take(8).collect();

    let canonical = run_slash_request(
        &format!("/contracts confirm {canonical_prefix}"),
        &runtime,
        &canonical_store,
    )
    .await;
    let alias = run_slash_request(&format!("/confirm {alias_prefix}"), &runtime, &alias_store).await;

    assert_eq!(canonical.0, alias.0, "one operation, two spellings");
    assert_eq!(canonical.2, 0, "/contracts confirm stderr: {}", canonical.4);
    assert_eq!(alias.2, 0, "/confirm stderr: {}", alias.4);

    let note = alias.1.expect("/confirm is a deprecated spelling");
    assert_eq!(canonical.1, None, "/contracts confirm states no note");
    // Same output, byte for byte, with the alias's one note line appended.
    assert_eq!(
        masked(&alias.3, &alias_id),
        format!("{}{note}\n", masked(&canonical.3, &canonical_id)),
        "the two spellings rendered differently"
    );
    assert_eq!(alias.3.matches(note).count(), 1, "the note is said once");
    assert!(
        !canonical.3.contains("deprecated"),
        "a canonical spelling says nothing about deprecation: {}",
        canonical.3
    );

    // Same store effect: both candidates are confirmed, both queues are empty,
    // and both stores list the same object.
    for (store, id, label) in [
        (&canonical_store, &canonical_id, "canonical"),
        (&alias_store, &alias_id, "alias"),
    ] {
        let (_cmd, code, out, err) = run_slash("/contracts queue", &runtime, store, RenderFormat::Text).await;
        assert_eq!(code, 0, "{label} queue stderr: {err}");
        assert!(!out.contains(id), "{label} queue still holds the confirmed claim: {out}");
    }
    let (_, _, canonical_list, _) =
        run_slash("/contracts", &runtime, &canonical_store, RenderFormat::Text).await;
    let (_, _, alias_list, _) =
        run_slash("/contracts", &runtime, &alias_store, RenderFormat::Text).await;
    assert_eq!(
        masked(&alias_list, &alias_id),
        masked(&canonical_list, &canonical_id),
        "the two stores did not end in the same state"
    );
    assert!(
        canonical_list.contains("analytics.public.orders"),
        "the confirmed claim is recallable: {canonical_list}"
    );

    let _ = fs::remove_dir_all(root);
}

/// `/contracts confirm-all` equals `/approve-all`: the same batch approve over
/// the same queue, and the deprecated spelling states its note once.
#[tokio::test]
async fn contracts_confirm_all_equals_the_deprecated_approve_all() {
    let root = temp_root("confirm_all_verb");
    let (runtime, _name) = runtime_at(&root);
    let canonical_store = store_at_name(&root, "canonical.sqlite3").await;
    let alias_store = store_at_name(&root, "alias.sqlite3").await;

    let canonical_id = seeded_candidate(&canonical_store, &runtime, "orders").await;
    let alias_id = seeded_candidate(&alias_store, &runtime, "orders").await;

    let canonical = run_slash_request("/contracts confirm-all --yes", &runtime, &canonical_store).await;
    let alias = run_slash_request("/approve-all --yes", &runtime, &alias_store).await;

    assert_eq!(canonical.0, alias.0, "one operation, two spellings");
    assert_eq!(canonical.2, 0, "/contracts confirm-all stderr: {}", canonical.4);
    assert_eq!(alias.2, 0, "/approve-all stderr: {}", alias.4);

    let note = alias.1.expect("/approve-all is a deprecated spelling");
    assert_eq!(
        masked(&alias.3, &alias_id),
        format!("{}{note}\n", masked(&canonical.3, &canonical_id)),
        "the two spellings rendered differently"
    );
    assert_eq!(alias.3.matches(note).count(), 1, "the note is said once");
    assert!(
        note.contains("/contracts confirm-all"),
        "the note names the new spelling: {note}"
    );

    // Both queues emptied: the batch approved the same set on both stores.
    for store in [&canonical_store, &alias_store] {
        let (_cmd, code, out, err) = run_slash("/contracts queue", &runtime, store, RenderFormat::Text).await;
        assert_eq!(code, 0, "queue stderr: {err}");
        assert!(!out.contains("ki-"), "the queue was approved: {out}");
    }

    let _ = fs::remove_dir_all(root);
}
