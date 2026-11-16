//! Diagnostics views over endpoints never carry a secret value: the
//! reference label is the contract, file references redact their path, and
//! base URLs redact their query — including strings inherited from the
//! plain `[ai]` block.

use saya_config::{ConfigFile, ConnectionsFile, ResolutionInput, resolve};

#[test]
fn endpoint_diagnostics_never_carry_a_secret_value() {
    const SECRET_VALUE: &str = "super-secret-value-424242";
    let user = ConfigFile::from_toml(
        "[ai]\napi_key = { env = 'SAYA_ENDPOINT_SECRET' }\n\
         [[ai.endpoints]]\nname = 'planner'\n\
         base_url = 'https://gateway.internal/v1?token=abc'\napi_key = { file = 'secrets.txt' }\n",
    )
    .unwrap();
    let resolved = resolve(
        ResolutionInput::new(ConnectionsFile::default())
            .with_user(user.clone())
            .with_process_env([("SAYA_ENDPOINT_SECRET", SECRET_VALUE)]),
    )
    .unwrap();
    let views = [
        serde_json::to_string(&user.redacted_diagnostics()).unwrap(),
        serde_json::to_string(&resolved.redacted_diagnostics()).unwrap(),
    ];
    for view in &views {
        assert!(
            !view.contains(SECRET_VALUE),
            "a resolved secret value leaked into a diagnostics view: {view}"
        );
        assert!(
            view.contains("env:SAYA_ENDPOINT_SECRET"),
            "the reference label is the view's contract: {view}"
        );
        assert!(
            view.contains("file:[redacted path]"),
            "file references redact the path: {view}"
        );
        assert!(
            view.contains("https://gateway.internal/v1?[redacted]"),
            "base_url redacts its query: {view}"
        );
    }
    assert!(
        views[0].contains("\"planner\""),
        "the file mirror names the declared endpoint"
    );
    assert!(
        views[1].contains("\"orchestrator\""),
        "the resolved mirror carries the orchestrator fallback"
    );
}

#[test]
fn inherited_endpoint_diagnostics_remain_redacted() {
    let user = ConfigFile::from_toml(
        "[ai]\nmodel = 'gateway-model'\nbase_url = 'https://user:pass@gateway.test/v1?token=secret'\n\
         api_key = { env = 'SAYA_KEY' }\n[[ai.endpoints]]\nname = 'planner'\n",
    )
    .unwrap();
    let resolved =
        resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(user)).unwrap();
    let planner = &resolved.redacted_diagnostics().endpoints["planner"];
    assert_eq!(planner.model.as_deref(), Some("gateway-model"));
    assert_eq!(
        planner.base_url.as_deref(),
        Some("https://gateway.test/v1?[redacted]")
    );
    assert_eq!(planner.api_key_reference.as_deref(), Some("env:SAYA_KEY"));
}
