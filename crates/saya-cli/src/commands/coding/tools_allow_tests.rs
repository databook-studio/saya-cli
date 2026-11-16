//! The coding job's own `[tools] allow` selection: what it persists, what an
//! older `request.json` still parses to, and what the operator is shown.
//!
//! A job that states no list must be indistinguishable from a job created
//! before the setting existed — same `request.json` bytes, same proposal bytes,
//! and therefore the same `policy_digest`. A job that states one carries it
//! through the resume.

use super::CodingJobRequest;
use super::request_fixture::{JOB, fixture, fixture_launched_with, fixture_with};
use std::fs;

/// The exact bytes the creation path writes for this fixture's job. It is the
/// pre-`[tools]` wire shape: the new key is absent, not null, so a job that
/// states no list is byte-identical to one written before the setting existed.
const DEFAULT_REQUEST_BYTES: &str = concat!(
    r#"{"job_id":"request-mode-legacy-resume","task":"resume a request written by an older binary","#,
    r#""write_paths":["tracked.txt"],"database_profiles":[],"database_identities":{}}"#
);

#[cfg(unix)]
#[tokio::test]
async fn a_default_job_writes_the_request_bytes_it_always_did() {
    let fixture = fixture("tools-default-bytes").await;
    let bytes = fs::read(fixture.job.records.join("request.json")).unwrap();
    assert_eq!(
        String::from_utf8(bytes).unwrap(),
        DEFAULT_REQUEST_BYTES,
        "a job that states no tools list must persist exactly today's bytes"
    );
}

/// The exact bytes the creation path shows for this fixture's job proposal,
/// taken from the parent `1045549f`. It is the pre-`[tools]` wire shape: the
/// key set, the key order, and every value the parent wrote. The three
/// `{plan}`/`{repository}`/`{path}` slots are the values that come from the
/// fixture's own temporary checkout and snapshot — read from the fixture, not
/// from the proposal — so this is an independent expectation rather than a
/// re-serialization of the value under test.
const DEFAULT_PROPOSAL: &str = concat!(
    r#"{"budget_mode":"bounded","database_origin":null,"database_profiles":{},"#,
    r#""job_id":"request-mode-legacy-resume","operation":"start","plan_sha256":"{plan}","#,
    r#""provider_bindings":{"child":{"context_bound_verified":false,"declared_context_window_tokens":null,"endpoint_fingerprint":"b8ad1e4f876a5e98df54563ba343627184cc5413de96b4bae55044c10544c5a8","idle_timeout_seconds":90,"max_output_tokens":null,"model_id":"qwen2.5-coder:14b","provider":"ollama","reasoning_effort":null,"retry_delays_ms":[500,1000,2000,4000,8000,16000,30000,30000],"source":"actual_native_provider_settings_projection","temperature":0.10000000149011612,"timeout_seconds":60},"#,
    r#""root":{"context_bound_verified":false,"declared_context_window_tokens":null,"endpoint_fingerprint":"b8ad1e4f876a5e98df54563ba343627184cc5413de96b4bae55044c10544c5a8","idle_timeout_seconds":90,"max_output_tokens":null,"model_id":"qwen2.5-coder:14b","provider":"ollama","reasoning_effort":null,"retry_delays_ms":[500,1000,2000,4000,8000,16000,30000,30000],"source":"actual_native_provider_settings_projection","temperature":0.10000000149011612,"timeout_seconds":60}},"#,
    r#""repository_id":"{repository}","repository_path":"{path}","revocation_epoch":1,"#,
    r#""snapshot_id":"request-mode-legacy-resume","task":"resume a request written by an older binary"}"#,
);

/// The parent's proposal bytes for this fixture, with the three
/// environment-derived slots filled from the fixture's own state.
fn expected_proposal(fixture: &super::request_fixture::Fixture) -> String {
    DEFAULT_PROPOSAL
        .replace("{plan}", fixture.job.plan.manifest_sha256().as_str())
        .replace("{repository}", fixture.job.repository_id.as_str())
        .replace(
            "{path}",
            fixture
                .application
                .root_path()
                .to_str()
                .expect("the fixture's repository path is UTF-8"),
        )
}

#[cfg(unix)]
#[tokio::test]
async fn a_default_job_proposal_is_the_bytes_the_parent_wrote() {
    let fixture = fixture("tools-default-proposal").await;
    let proposal = fixture
        .application
        .propose(&fixture.job.request)
        .await
        .unwrap();
    assert!(
        proposal.request().get("tools_allow").is_none(),
        "a job that states no list shows no key: {}",
        proposal.request()
    );
    let expected = expected_proposal(&fixture);
    assert_eq!(
        serde_json::to_string(proposal.request()).unwrap(),
        expected,
        "a job that states no list must propose exactly the parent's bytes"
    );
    // And the digest the job stores is taken over those bytes: an absent key is
    // what keeps a default job's `policy_digest` identical to the parent's.
    assert_eq!(
        fixture.job.policy_digest,
        super::create::digest(expected.as_bytes()).unwrap(),
        "the job's policy digest is the digest of the proposal bytes shown"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_stated_list_is_persisted_canonically_and_resumes_with_it() {
    let fixture = fixture_with("tools-stated", |request| {
        request
            .with_tools_allow(Some(vec!["bash".into(), "read".into()]))
            .unwrap()
    })
    .await;
    let bytes = fs::read(fixture.job.records.join("request.json")).unwrap();
    let written = String::from_utf8(bytes).unwrap();
    assert!(
        written.contains(r#""tools_allow":["read","shell"]"#),
        "the list is persisted canonical and sorted: {written}"
    );
    // Resuming replays the request from disk: the same set, not whatever the
    // surface resolves today.
    let resumed = fixture
        .application
        .continuation_request(fixture.job.job_id())
        .unwrap();
    assert_eq!(
        resumed.tools_allow.as_deref(),
        Some(&["read".to_string(), "shell".to_string()][..])
    );
    let tools = resumed.tools().unwrap();
    assert!(tools.admits("read"));
    assert!(tools.admits("bash"), "the alias names the same tool");
    assert!(!tools.admits("write"));
}

#[cfg(unix)]
#[tokio::test]
async fn an_older_request_json_without_a_tools_list_still_parses_and_gets_the_default() {
    // The bytes an older binary wrote: no `tools_allow` at all.
    let bytes = br#"{"job_id":"request-mode-legacy-resume","task":"resume a request written by an older binary","write_paths":["tracked.txt"],"programs":[],"database_profiles":[],"database_identities":{}}"#;
    let request: CodingJobRequest = serde_json::from_slice(bytes).unwrap();
    request.validate().unwrap();
    assert_eq!(request.job_id().as_str(), JOB);
    let tools = request.tools().unwrap();
    assert!(
        tools.allow.is_none(),
        "an older request narrows nothing, exactly as it did when it was written"
    );
    for name in saya_config::GOVERNED_TOOLS {
        assert!(tools.admits(name));
    }
}

#[cfg(unix)]
#[tokio::test]
async fn a_request_naming_a_tool_the_list_cannot_govern_is_refused() {
    for words in [
        vec!["nope".to_string()],
        vec!["schema_discovery".to_string()],
        Vec::new(),
    ] {
        let shaped = CodingJobRequest::new(
            saya_types::JobId::parse(JOB).unwrap(),
            "bounded task".into(),
            vec![],
        )
        .unwrap()
        .with_tools_allow(Some(words.clone()));
        assert!(
            shaped.is_err(),
            "{words:?} must be refused rather than silently ignored"
        );
    }
}

/// Inside a session, `/start --tools` may only narrow the session's own
/// list. The CLI flag is the top layer and replaces the config; a session has
/// no launch to replace, so a name the session does not advertise is a typed
/// error naming it — never a silent widening of the job's surface.
#[cfg(unix)]
#[tokio::test]
async fn a_session_coding_start_may_only_narrow_the_session_list() {
    let fixture = fixture_launched_with("tools-session-narrowing", &["grep", "read"]).await;
    // No flag: the job gets the session's list.
    assert_eq!(
        fixture.application.stated_tools(&[]).unwrap(),
        Some(vec!["grep".to_string(), "read".to_string()]),
        "a start that states nothing runs the session's set"
    );
    // A narrower list is accepted, and is exactly what the job gets.
    assert_eq!(
        fixture
            .application
            .stated_tools(&["read".to_string()])
            .unwrap(),
        Some(vec!["read".to_string()]),
        "a name inside the session's list narrows the job"
    );
    // A name the session does not advertise is refused, and the refusal names
    // only the tools that widened.
    let error = fixture
        .application
        .stated_tools(&["read".to_string(), "write".to_string()])
        .expect_err("`write` is not in this session's list");
    let text = error.to_string();
    assert!(
        text.contains("\"write\""),
        "the refusal names the tool: {text}"
    );
    assert!(
        !text.contains("\"read\""),
        "only the widening names are reported: {text}"
    );
    assert!(
        text.contains("grep") && text.contains("read"),
        "the refusal shows the session's own list: {text}"
    );
}

/// The other half of the same rule: a session that states no list admits
/// everything, so a `/start --tools` narrows rather than widens — and
/// there is nothing outside the session's set to refuse.
#[cfg(unix)]
#[tokio::test]
async fn a_session_with_no_list_admits_a_stated_narrowing() {
    let fixture = fixture("tools-session-unstated").await;
    assert_eq!(
        fixture
            .application
            .stated_tools(&["write".to_string()])
            .unwrap(),
        Some(vec!["write".to_string()]),
        "an unstated session list admits the stated narrowing"
    );
}

/// The launch flag reaches the job. `saya --tools read start …` and
/// `saya start … --tools read` are the same one global list, resolved
/// into the runtime; a start that states no words of its own persists it.
#[cfg(unix)]
#[tokio::test]
async fn the_launch_flag_reaches_the_job() {
    let fixture = fixture_launched_with("tools-launch-flag", &["read"]).await;
    assert_eq!(
        fixture.application.stated_tools(&[]).unwrap(),
        Some(vec!["read".to_string()]),
        "the global flag is the job's list"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_narrowed_job_states_its_set_in_the_proposal() {
    let fixture = fixture_with("tools-proposal", |request| {
        request
            .with_tools_allow(Some(vec!["read".into(), "grep".into()]))
            .unwrap()
    })
    .await;
    let proposal = fixture
        .application
        .propose(&fixture.job.request)
        .await
        .unwrap();
    assert_eq!(
        proposal.request().get("tools_allow"),
        Some(&serde_json::json!(["grep", "read"])),
        "the operator approves the tool surface as well as the task: {}",
        proposal.request()
    );
    // And the digest still follows the bytes actually shown.
    let expected = super::create::digest(&serde_json::to_vec(proposal.request()).unwrap()).unwrap();
    assert_eq!(fixture.job.policy_digest, expected);
}
