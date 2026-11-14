#[test]
fn query_readiness_equals_can_run_query_when_ready() {
    let runtime = query_runtime(Some("type = \"sqlite\"\npath = \"/tmp/x.sqlite\"\n"));
    let readiness = readiness(&runtime, DoctorTask::Query, Path::new("/"));
    assert_eq!(readiness.ready, doctor::report(&runtime).can_run_query);
    assert!(readiness.ready);
    let profile = check_named(&readiness, "profile");
    assert_eq!(profile.status, CheckStatus::Ok);
    assert!(
        profile.detail.contains("selected: analytics (sqlite)"),
        "{profile:?}"
    );
    assert_eq!(check_named(&readiness, "secrets").status, CheckStatus::Ok);
}

#[test]
fn query_readiness_equals_can_run_query_when_a_secret_does_not_resolve() {
    let runtime = query_runtime(Some(
        "type = \"postgresql\"\nhost = \"localhost\"\ndatabase = \"warehouse\"\n\
         user = \"u\"\npassword = { env = \"SAYA_TEST_UNRESOLVED\" }\n",
    ));
    let readiness = readiness(&runtime, DoctorTask::Query, Path::new("/"));
    assert_eq!(readiness.ready, doctor::report(&runtime).can_run_query);
    assert!(!readiness.ready);
    let secrets = check_named(&readiness, "secrets");
    assert_eq!(secrets.status, CheckStatus::Fail);
    assert!(
        secrets.detail.contains("env:SAYA_TEST_UNRESOLVED"),
        "{secrets:?}"
    );
}

#[test]
fn query_readiness_equals_can_run_query_when_no_profile_is_selected() {
    let runtime = query_runtime(None);
    let readiness = readiness(&runtime, DoctorTask::Query, Path::new("/"));
    assert_eq!(readiness.ready, doctor::report(&runtime).can_run_query);
    assert!(!readiness.ready);
    let profile = check_named(&readiness, "profile");
    assert_eq!(profile.status, CheckStatus::Fail);
    assert_eq!(
        profile.detail,
        "no connection profile is selected — add one to connections.toml or pass --connections/--profile"
    );
    // No selected profile means no references to resolve: vacuously fine.
    assert_eq!(check_named(&readiness, "secrets").status, CheckStatus::Ok);
}
