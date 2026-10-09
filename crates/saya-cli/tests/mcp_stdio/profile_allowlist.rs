/// The audit reproduction (A922-1, D1): while one slow replay holds the
/// replay slot, a second replay is queued whose binding the ordinary CLI
/// then remaps (`investigation run <id> --connection other --revalidate`).
/// When the slot frees, the queued call must never execute what the binding
/// now names: the target is resolved once inside the serialized section and
/// refused there. The old flow gated the binding before the wait and passed
/// the target through, so the remapped profile executed and returned the
/// other database's rows.
#[test]
fn mcp_investigation_run_replay_binds_one_authorized_target() {
    const SENTINEL: &str = "OTHER-SYNTHETIC-SENTINEL-9";
    let fixture = crate_fixture("replay-remap");
    fixture.seed_events();
    fixture.seed_sentinel(SENTINEL);
    let mut config = std::fs::read_to_string(&fixture.config).unwrap();
    config.push_str("\n[run]\nquery_timeout_seconds = 6\n");
    std::fs::write(&fixture.config, config).unwrap();
    // Both investigations are saved against `local` (save never connects,
    // so the sentinel read saves cleanly against the wrong database); the
    // slow one holds the replay slot, the other is the queued call.
    let slow_id = fixture.save_investigation("slow sum", SLOW_SQL, "local");
    let remap_id =
        fixture.save_investigation("sentinel read", "SELECT label FROM sentinel", "local");

    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );
    server.handshake();

    // (1) The slow replay is in flight and holds the replay slot.
    server.send(&format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"investigation_run","arguments":{{"id":"{slow_id}"}}}}}}"#
    ));
    // (2) The queued replay arrives while the slot is held; its binding
    // still names `local`, so anything read at admission time passes.
    server.send(&format!(
        r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"investigation_run","arguments":{{"id":"{remap_id}"}}}}}}"#
    ));
    // (3) The ordinary CLI remaps the queued investigation to `other` — the
    // exact sequence the audit drove — and finishes long before the slow
    // replay's query timeout frees the slot.
    let (code, out, err) = fixture.cli(&[
        "investigation",
        "run",
        &remap_id,
        "--connection",
        "other",
        "--revalidate",
    ]);
    assert_eq!(code, 0, "the CLI rebind must succeed: {out}{err}");

    // (4) The slow replay times out first; its answer leaves the wire
    // before the queued call's.
    let first = server.next_json(60);
    assert_eq!(first["id"], 2);

    // (5) The queued call resolves inside the serialized section; the
    // binding now names a profile outside the allowlist: refused there,
    // never executed — the sentinel row never leaves its database.
    let second = server.next_json(60);
    assert_eq!(second["id"], 3);
    assert_eq!(
        second["result"]["isError"],
        true,
        "the remapped binding is refused: {}",
        serde_json::to_string(&second).unwrap()
    );
    let text = second["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        text.contains("profile not available"),
        "the refusal names the gate: {text}"
    );
    assert!(
        !text.contains(SENTINEL),
        "the non-allowlisted database is never read: {text}"
    );
    assert!(
        !text.contains("other"),
        "the binding's profile is not echoed: {text}"
    );

    server.close_and_expect_exit(0);
}

/// The state DB's audited profile identities, read through the store's own
/// API (the pool opens and migrates lazily, so a call before any audit is
/// empty, not a missing table). Every replay execution writes one audit row
/// — success or failure — before the answer returns, so an empty list after
/// a call means nothing executed against any profile.
fn audit_profiles(state: &std::path::Path) -> Vec<String> {
    use saya_store::{AuditStore, SqliteStateStore};
    let store = SqliteStateStore::new(state.to_path_buf());
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { store.recent_audit(100).await.unwrap() })
        .into_iter()
        .map(|record| record.event.profile_id)
        .collect()
}

/// `investigation_run` never reaches a profile outside the startup
/// allowlist (F-1) — neither through the `profile` argument nor through a
/// saved binding's profile. The refusal happens before anything runs: the
/// non-allowlisted database keeps its sentinel to itself and no audit row
/// exists anywhere. A binding inside the allowlist still runs.
#[test]
fn mcp_investigation_run_stays_inside_the_profile_allowlist() {
    const SENTINEL: &str = "OTHER-ONLY-SENTINEL-7";
    let fixture = crate_fixture("replay-allowlist");
    fixture.seed_events();
    fixture.seed_sentinel(SENTINEL);
    let local_id =
        fixture.save_investigation("local count", "SELECT count(*) AS n FROM events", "local");
    let other_id = fixture.save_investigation("other count", "SELECT label FROM sentinel", "other");

    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );
    server.handshake();
    assert!(
        audit_profiles(&fixture.state).is_empty(),
        "nothing has audited yet"
    );

    // (a) The binding points at `other`, which the server does not serve:
    // refused before anything runs — no audit row exists anywhere, and
    // neither the binding's profile name nor the sentinel value is echoed.
    let text = server.call_error_text(2, "investigation_run", &format!(r#"{{"id":"{other_id}"}}"#));
    assert!(
        text.contains("profile not available"),
        "the non-allowlisted binding is refused: {text}"
    );
    assert!(
        !text.contains("other"),
        "the binding's profile name is not echoed: {text}"
    );
    assert!(
        !text.contains(SENTINEL),
        "the non-allowlisted database is never read: {text}"
    );
    assert!(
        audit_profiles(&fixture.state).is_empty(),
        "a refusal writes no audit row: {:?}",
        audit_profiles(&fixture.state)
    );

    // (b) An explicit `profile` argument naming the non-allowlisted profile
    // is refused the same way — even though this investigation's binding is
    // allowlisted.
    let text = server.call_error_text(
        3,
        "investigation_run",
        &format!(r#"{{"id":"{local_id}","profile":"other"}}"#),
    );
    assert!(
        text.contains("profile not available") && text.contains("other"),
        "the requested name is echoed: {text}"
    );
    assert!(
        audit_profiles(&fixture.state).is_empty(),
        "a refusal writes no audit row: {:?}",
        audit_profiles(&fixture.state)
    );

    // (c) A binding inside the allowlist runs.
    let run = server.call(4, "investigation_run", &format!(r#"{{"id":"{local_id}"}}"#));
    assert_eq!(
        run["result"]["isError"],
        false,
        "{}",
        serde_json::to_string(&run).unwrap()
    );
    assert_eq!(run["result"]["structuredContent"]["connection"], "local");
    let audited = audit_profiles(&fixture.state);
    assert!(
        !audited.is_empty(),
        "the allowed replay audited: {audited:?}"
    );

    server.close_and_expect_exit(0);
}
