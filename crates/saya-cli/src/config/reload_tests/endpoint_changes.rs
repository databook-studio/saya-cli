#[test]
fn an_unbound_endpoint_follows_the_new_configuration() {
    let old = runtime_from("[ai]\nbase_url = \"http://old:8080/v1\"\n", "");
    let new = runtime_from("[ai]\nbase_url = \"http://new:8080/v1\"\n", "");
    let plan = plan_reload(&old, &new, &state_with("m"));
    assert_eq!(plan.reseeded[0].key, "base_url");
    assert_eq!(
        plan.reseed.provider_endpoint,
        Some(Some("http://new:8080/v1".to_owned()))
    );
    let mut session = state_with("m");
    apply_plan(&plan, &mut session);
    assert_eq!(
        session.provider_endpoint.as_deref(),
        Some("http://new:8080/v1")
    );
    assert!(session.provider_endpoint_bound);
}

#[test]
fn an_overridden_endpoint_stays_pending() {
    let old = runtime_from("[ai]\nbase_url = \"http://old:8080/v1\"\n", "");
    let new = runtime_from("[ai]\nbase_url = \"http://new:8080/v1\"\n", "");
    let mut session = state_with("m");
    session.provider_endpoint = Some("http://user:9/v1".to_owned());
    session.provider_endpoint_bound = true;
    let plan = plan_reload(&old, &new, &session);
    assert_eq!(plan.pending.len(), 1);
    assert_eq!(plan.pending[0].key, "base_url");
    assert_eq!(plan.reseed, StateReseed::default());
    let mut after = session.clone();
    apply_plan(&plan, &mut after);
    assert_eq!(after.provider_endpoint.as_deref(), Some("http://user:9/v1"));
}

#[test]
fn an_endpoint_already_at_the_new_value_is_omitted_from_the_reseed() {
    let old = runtime_from("[ai]\nbase_url = \"http://old:8080/v1\"\n", "");
    let new = runtime_from("[ai]\nbase_url = \"http://new:8080/v1\"\n", "");
    let mut session = state_with("m");
    session.provider_endpoint = Some("http://new:8080/v1".to_owned());
    session.provider_endpoint_bound = true;
    let plan = plan_reload(&old, &new, &session);
    assert!(
        !plan
            .applied
            .iter()
            .chain(&plan.pending)
            .chain(&plan.reseeded)
            .any(|line| line.key == "base_url")
    );
    assert_eq!(plan.reseed.provider_endpoint, None);
}
