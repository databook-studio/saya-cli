use super::*;

#[test]
fn the_session_policy_allows_any_https_host_and_keeps_the_structural_gates() {
    let session = FetchPolicy::session();
    // Any declared-or-not public host is fetched: consent is per call.
    session
        .allow_url("https://api.github.com/repos/saya/saya")
        .expect("an undeclared public host fetches under the session policy");
    session
        .allow_url("https://example.org/")
        .expect("a second host too — the list is not the gate");
    // The structural gates are unchanged, declared or not.
    assert!(session.allow_url("http://example.org/").is_err());
    assert!(session.allow_url("https://127.0.0.1/").is_err());
    assert!(session.allow_url("https://169.254.169.254/").is_err());
    assert!(session.allow_url("https://localhost/").is_err());
    assert!(session.allow_url("https://10.1.2.3/").is_err());
    assert!(session.allow_url("not a url").is_err());
}

#[test]
fn the_declared_policy_still_refuses_undeclared_hosts() {
    // A run's policy is untouched by the session constructor: an undeclared
    // host still refuses — the session shape never leaks into the run path.
    let declared = policy();
    must_refuse("https://undeclared.example.org/");
    let _ = declared;
}
