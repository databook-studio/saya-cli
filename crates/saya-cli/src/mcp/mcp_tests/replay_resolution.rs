/// The replay's target resolution (F-1, D1): the effective target — the
/// `profile` argument, else the saved binding's profile — is resolved once,
/// inside the serialized replay section, and returned as the concrete
/// target the command will carry. A target outside the allowlist is refused
/// with a binding's name never echoed; a client-supplied name is refused
/// with the name it asked for; and with neither an argument nor a binding —
/// or a binding that cannot be read — the replay refuses `profile not
/// available`: there is no pass-through that lets the run command resolve
/// its own target at execution time (A922-1 — a binding read after a queued
/// wait can be remapped underneath the call).
#[test]
fn the_replay_resolution_binds_one_authorized_target() {
    fn binding(id: &InvestigationId, profile: &str) -> LocalBinding {
        LocalBinding {
            version: LocalBinding::VERSION,
            id: id.clone(),
            profile: profile.to_owned(),
            profile_identity: "identity".to_owned(),
            reviewed_revision: 1,
            reviewed_schema_fingerprint: None,
            reviewed_unix_ms: 0,
        }
    }

    let root = std::env::temp_dir().join(format!("saya-mcp-gate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mut runtime = fixture_runtime(TWO_PROFILES, "default_profile = 'first'\n");
    runtime.investigations_root = root.clone();
    let repo = InvestigationRepository::new(root);
    let other_bound = InvestigationId::parse("bound-second-1").unwrap();
    let local_bound = InvestigationId::parse("bound-first-1").unwrap();
    repo.put_binding(&binding(&other_bound, "second")).unwrap();
    repo.put_binding(&binding(&local_bound, "first")).unwrap();

    let context = super::context::McpContext {
        runtime: runtime.clone(),
        store: saya_store::SqliteStateStore::new(std::env::temp_dir().join("saya-mcp-test-state")),
        replay_slot: tokio::sync::Mutex::new(()),
    };
    let policy = ServePolicy::resolve(&runtime, &["first".to_owned()], None).unwrap();
    use super::replay_tools::resolve_target;

    // A binding inside the allowlist resolves to the concrete target the
    // command will carry — not a pass-through verdict.
    let target = resolve_target(&policy, &context, local_bound.as_str(), None)
        .expect("a binding inside the allowlist resolves");
    assert_eq!(
        target, "first",
        "the resolution is the binding's profile, returned concretely"
    );
    let target = resolve_target(&policy, &context, local_bound.as_str(), Some("first"))
        .expect("an allowlisted argument resolves");
    assert_eq!(target, "first", "the argument is the target");

    // A binding pointing outside the allowlist: refused, the name not echoed.
    let refused = resolve_target(&policy, &context, other_bound.as_str(), None)
        .expect_err("a binding outside the allowlist is refused");
    assert!(
        refused.contains("profile not available"),
        "the refusal names the gate: {refused}"
    );
    assert!(
        !refused.contains("second"),
        "the binding's profile is never echoed: {refused}"
    );

    // An argument naming the same profile: refused with the requested name.
    let refused = resolve_target(&policy, &context, other_bound.as_str(), Some("second"))
        .expect_err("a non-allowlisted argument is refused");
    assert!(
        refused.contains("profile not available") && refused.contains("second"),
        "the requested name is echoed: {refused}"
    );

    // Neither an argument nor a binding — and a malformed id: the replay
    // refuses at the MCP layer. There is no pass-through (D1): a target the
    // MCP layer could not name is not something the run may resolve on its
    // own at execution time.
    let refused = resolve_target(&policy, &context, "no-such-id", None)
        .expect_err("a missing binding refuses");
    assert!(
        refused.contains("profile not available"),
        "the refusal names the gate: {refused}"
    );
    let refused = resolve_target(&policy, &context, "REFUSED-ID", None)
        .expect_err("a malformed id has no resolvable target");
    assert!(
        refused.contains("profile not available"),
        "the refusal names the gate: {refused}"
    );

    // A binding that exists but cannot be read fails the resolution CLOSED:
    // the replay does not start, and neither the store error's own text nor
    // the bound profile name is echoed.
    let corrupt = InvestigationId::parse("corrupt-binding-1").unwrap();
    let corrupt_path = runtime
        .investigations_root
        .join("local")
        .join(format!("{}.json", corrupt.as_str()));
    std::fs::write(&corrupt_path, b"not json at all").unwrap();
    let refused = resolve_target(&policy, &context, corrupt.as_str(), None)
        .expect_err("an unreadable binding is refused");
    assert!(
        refused.contains("profile not available"),
        "the refusal names the gate: {refused}"
    );
    assert!(
        !refused.contains("not valid for storage") && !refused.contains("second"),
        "neither the store error nor a profile name is echoed: {refused}"
    );
}
