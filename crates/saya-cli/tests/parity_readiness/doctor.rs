use super::support::{Fixture, application, blocked, optional};
use std::fs;

/// Doctor and setup read the same readiness collection and must name the same
/// non-blocker: a coding configuration with no installed MCP/LSP extension
/// profiles is ready, and both surfaces list the unconfigured profiles under
/// `optional` — never under `blocked` — without touching the repository.
#[tokio::test]
async fn doctor_and_setup_report_missing_extension_profiles_as_optional() {
    let fixture = Fixture::new("parity-missing-extension").await;
    let app = application(&fixture).await;
    let doctor = app.doctor(false).await.unwrap();
    let setup = app.setup(false).await.unwrap();
    assert_eq!(doctor["ready"], true);
    assert_eq!(setup["ready"], true);
    for (name, report) in [("doctor", &doctor), ("setup", &setup)] {
        assert!(
            blocked(report).is_empty(),
            "{name} must not block an unconfigured optional profile: {report}"
        );
        assert!(
            optional(report)
                .iter()
                .any(|message| message.contains("MCP and LSP profiles")),
            "{name} must name the unconfigured profiles as optional: {report}"
        );
    }
    assert_eq!(
        fs::read(fixture.repository.join("tracked.txt")).unwrap(),
        b"original\n"
    );
}
