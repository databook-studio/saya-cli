#[test]
fn live_changes_land_in_applied_only() {
    let old = runtime_from(
        "[ai]\nmodel = \"m1\"\n[run]\nmax_rows = 50\nread_only = true\n",
        "",
    );
    let new = runtime_from(
        "[ai]\nmodel = \"m1\"\n[run]\nmax_rows = 100\nread_only = false\n",
        "",
    );
    let plan = plan_reload(&old, &new, &state_with("m1"));
    assert!(plan.reseeded.is_empty() && plan.pending.is_empty());
    let keys: Vec<&str> = plan.applied.iter().map(|line| line.key.as_str()).collect();
    assert_eq!(keys, ["max_rows", "read_only"]);
    assert_eq!(plan.reseed, StateReseed::default());
    assert_eq!(
        plan.applied
            .iter()
            .find(|l| l.key == "max_rows")
            .unwrap()
            .after,
        json!(100)
    );
}

#[test]
fn a_never_overridden_model_is_reseeded() {
    let old = runtime_from("[ai]\nmodel = \"m1\"\n", "");
    let new = runtime_from("[ai]\nmodel = \"m2\"\n", "");
    let plan = plan_reload(&old, &new, &state_with("m1"));
    assert!(plan.pending.is_empty());
    assert_eq!(plan.reseeded[0].key, "model");
    assert_eq!(plan.reseed.model.as_deref(), Some("m2"));
    let keys: Vec<&str> = plan.applied.iter().map(|line| line.key.as_str()).collect();
    assert_eq!(
        keys,
        ["endpoints"],
        "the orchestrator endpoint mirrors [ai]"
    );
    let mut session = state_with("m1");
    apply_plan(&plan, &mut session);
    assert_eq!(session.model, "m2");
}

#[test]
fn a_never_overridden_provider_is_reseeded() {
    let old = runtime_from("", "");
    let new = runtime_from("[ai]\nprovider = \"openai\"\n", "");
    let plan = plan_reload(&old, &new, &state_with("m"));
    assert_eq!(plan.reseeded[0].key, "provider");
    assert_eq!(plan.reseed.provider.as_deref(), Some("openai"));
    let mut session = state_with("m");
    apply_plan(&plan, &mut session);
    assert_eq!(session.provider, "openai");
}

#[test]
fn an_overridden_provider_stays_pending_and_the_state_is_untouched() {
    let old = runtime_from("", "");
    let new = runtime_from("[ai]\nprovider = \"openai\"\n", "");
    let mut session = state_with("m");
    session.provider = "anthropic".to_owned();
    let plan = plan_reload(&old, &new, &session);
    assert!(plan.reseeded.is_empty());
    assert_eq!(plan.pending.len(), 1);
    assert_eq!(plan.pending[0].key, "provider");
    assert_eq!(
        plan.pending[0].note.as_deref(),
        Some("this session overrides it (use /provider, /model or /connect to change it here)")
    );
    let keys: Vec<&str> = plan.applied.iter().map(|line| line.key.as_str()).collect();
    assert_eq!(
        keys,
        ["endpoints"],
        "only the endpoint pool mirrors the change"
    );
    assert_eq!(plan.reseed, StateReseed::default());
    let mut after = session.clone();
    apply_plan(&plan, &mut after);
    assert_eq!(after.provider, "anthropic");
}

#[test]
fn a_provider_already_at_the_new_value_is_omitted_from_the_reseed() {
    let old = runtime_from("", "");
    let new = runtime_from("[ai]\nprovider = \"openai\"\n", "");
    let mut session = state_with("m");
    session.provider = "openai".to_owned();
    let plan = plan_reload(&old, &new, &session);
    assert!(
        !plan.applied.is_empty() || !plan.pending.is_empty() || !plan.reseeded.is_empty(),
        "the endpoint pool still changed, so the plan is not empty"
    );
    assert!(
        !plan
            .applied
            .iter()
            .chain(&plan.pending)
            .chain(&plan.reseeded)
            .any(|line| line.key == "provider")
    );
    assert_eq!(plan.reseed.provider, None);
}
