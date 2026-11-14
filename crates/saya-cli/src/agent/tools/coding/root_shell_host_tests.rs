//! The root shell's host lane: a host with no proven sandbox treats
//! every call as a host call; a host run inherits this process's environment
//! minus the names the host policy removes, and its result says it ran
//! outside the sandbox; a sandboxed run still sees none of the parent's
//! environment at all.

use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    process::{Command, Stdio},
};

use saya_agent::{ApprovalDecider, ApprovalPolicy};
use saya_harness::runner::HostPolicy;

use super::definition;
use super::fixture::{
    NO_OPERATOR_REFUSAL, RecordingOperator, TempCheckout, call, host_call, host_shell_over,
    sandbox_call, shell_over,
};

/// The canary root the outer run points its inner run at.
const CANARY_ROOT: &str = "SAYA_ROOT_SHELL_CANARY_ROOT";
/// The inner run that dumps the host child's environment.
const HOST_ENV_TEST: &str =
    "agent::tools::coding::root_shell_tests::host::a_host_run_does_not_inherit_saya_secrets";
/// The inner run that dumps the sandboxed child's environment.
const SANDBOX_ENV_TEST: &str = "agent::tools::coding::root_shell_tests::host::a_parent_variable_never_reaches_the_sandboxed_shell";
/// The configured provider key the host run must not inherit.
const PROVIDER_KEY: &str = "SAYA_C2B_PROVIDER_KEY";

/// (f) With no proven sandbox the shell holds only the host lane, so every
/// call — `sandbox` omitted, false, or even true — is a host call and reaches
/// the mode.
#[tokio::test]
async fn a_host_with_no_proven_sandbox_treats_every_call_as_a_host_call() {
    let checkout = TempCheckout::new("host-only");
    let operator = RecordingOperator::new(true);
    let Some(shell) = host_shell_over(
        checkout.path(),
        ApprovalPolicy::Ask,
        Some(operator.clone()),
        HostPolicy::default(),
    ) else {
        return;
    };
    assert!(shell.approve(&definition(), &call("true")).await);
    assert_eq!(
        operator.calls(),
        1,
        "a call naming no sandbox is a host call"
    );
    assert!(shell.approve(&definition(), &host_call("true")).await);
    assert_eq!(operator.calls(), 2);
    assert!(shell.approve(&definition(), &sandbox_call("true")).await);
    assert_eq!(
        operator.calls(),
        3,
        "there is no sandbox lane, so even `sandbox: true` is a host call"
    );

    let ran = shell
        .call("shell", &call("true"))
        .await
        .expect("the host lane runs the call");
    assert_eq!(
        ran["sandboxed"], false,
        "the host lane reports itself: {ran}"
    );

    let Some(shell) = host_shell_over(
        checkout.path(),
        ApprovalPolicy::Ask,
        None,
        HostPolicy::default(),
    ) else {
        return;
    };
    assert!(!shell.approve(&definition(), &call("true")).await);
    assert_eq!(
        shell
            .refusal_detail(&definition(), &call("true"))
            .as_deref(),
        Some(NO_OPERATOR_REFUSAL)
    );
}

/// A host run inherits this process's environment minus the names the policy
/// removes: a planted `SAYA_` secret and a configured provider key variable
/// never reach the child, while an ordinary variable does.
#[tokio::test]
async fn a_host_run_does_not_inherit_saya_secrets() {
    if let Some(dir) = std::env::var_os(CANARY_ROOT) {
        inner_host_env(PathBuf::from(dir)).await;
        return;
    }
    let guard = TempCheckout::new("host-env-outer");
    let status = Command::new(std::env::current_exe().expect("the test binary path resolves"))
        .arg("--exact")
        .arg(HOST_ENV_TEST)
        .env(CANARY_ROOT, guard.path())
        .env("SAYA_X_SECRET", "saya-planted-secret")
        .env(PROVIDER_KEY, "provider-planted-secret")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("the self-re-exec must spawn");
    assert!(
        status.success(),
        "the inner host-env test must pass: {status}"
    );
    let text = fs::read_to_string(guard.path().join("host-env.txt"))
        .expect("the inner run must record the host child's env");
    for planted in ["saya-planted-secret", "provider-planted-secret"] {
        assert!(
            !text.contains(planted),
            "the host child must not inherit a saya secret: {text}"
        );
    }
    for name in ["SAYA_X_SECRET=", "SAYA_C2B_PROVIDER_KEY="] {
        assert!(
            !text.contains(name),
            "the host child must not even carry the name {name}: {text}"
        );
    }
    assert!(
        text.contains("PATH="),
        "the host child inherits the rest of this process's environment: {text}"
    );
}

/// The inner half of [`a_host_run_does_not_inherit_saya_secrets`]: builds the
/// host policy over a config that names the provider key, then dumps the host
/// child's environment into the canary root.
async fn inner_host_env(dir: PathBuf) {
    use super::super::shell_secrets::host_policy;
    let config_dir = TempCheckout::new("host-env-config");
    let checkout = TempCheckout::new("host-env-checkout");
    fs::write(
        config_dir.path().join("config.toml"),
        format!("[ai]\napi_key = {{ env = \"{PROVIDER_KEY}\" }}\n"),
    )
    .expect("the config file must be writable");
    let process: BTreeMap<String, String> = std::env::vars().collect();
    let runtime = crate::load_with_sources(
        &crate::GlobalOptions::default(),
        config_dir.path(),
        config_dir.path(),
        process,
    )
    .expect("the fixture config resolves");
    // `bypass` is the mode that runs a host call with no operator to ask: the
    // inner run is about what the host child inherits, not about the asking.
    let Some(shell) = host_shell_over(
        checkout.path(),
        ApprovalPolicy::Bypass,
        None,
        host_policy(&runtime),
    ) else {
        fs::write(dir.join("host-env.txt"), b"unproven").expect("record the failure");
        return;
    };
    let outcome = shell
        .call("shell", &call("env"))
        .await
        .expect("the env dump must run");
    fs::write(
        dir.join("host-env.txt"),
        outcome["stdout"].as_str().unwrap_or_default(),
    )
    .expect("record the child's env");
}

/// The sandboxed lane is unchanged: the parent's environment never reaches
/// the child, which sees only the declared four. The canary is planted in the
/// inner process's environment by the outer one, so no `set_var` ever touches
/// the shared test binary's own environment.
#[tokio::test]
async fn a_parent_variable_never_reaches_the_sandboxed_shell() {
    if let Some(dir) = std::env::var_os(CANARY_ROOT) {
        let dir = PathBuf::from(dir);
        let checkout = TempCheckout::new("canary-inner");
        let Some(shell) = shell_over(checkout.path(), ApprovalPolicy::Ask, None) else {
            fs::write(dir.join("unproven"), b"1").expect("record the unproven host");
            return;
        };
        let outcome = shell
            .call("shell", &call("env"))
            .await
            .expect("the env dump must run");
        fs::write(
            dir.join("sandbox-env.txt"),
            outcome["stdout"].as_str().unwrap_or_default(),
        )
        .expect("record the child's env");
        return;
    }

    let guard = TempCheckout::new("canary-outer");
    let status = Command::new(std::env::current_exe().expect("the test binary path resolves"))
        .arg("--exact")
        .arg(SANDBOX_ENV_TEST)
        .env(CANARY_ROOT, guard.path())
        .env("SAYA_ROOT_SHELL_CANARY", "leak-me")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("the self-re-exec must spawn");
    assert!(
        status.success(),
        "the inner canary test must pass: {status}"
    );
    if guard.path().join("unproven").exists() {
        eprintln!("skipping `canary`: the sandbox is not proven on this host");
        return;
    }
    let text = fs::read_to_string(guard.path().join("sandbox-env.txt"))
        .expect("the inner run must record the env");
    assert!(
        !text.contains("leak-me"),
        "the parent's variable must not reach the shell child: {text}"
    );
    assert!(
        text.contains("PATH=") && text.contains("TERM=dumb"),
        "the child's environment is the declared four, and nothing else: {text}"
    );
}
