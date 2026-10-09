//! The child's environment: nothing the parent carries is inherited, and
//! only an explicitly passed variable reaches the child.

use super::{config, leak, stage};

use saya_harness::host::HostCommand;

#[tokio::test]
async fn a_planted_parent_env_var_never_reaches_the_child() {
    unsafe { std::env::set_var("SAYA_HOST_PROBE_PLANTED", "hunter2-planted") };
    let dir = leak("env");
    stage(&dir, "env-probe");
    let config = config(&dir.display().to_string(), &[]);
    let outcome = HostCommand::new("env-probe", ["env".to_owned()])
        .expect("shaped")
        .run(&config, None, &saya_agent::CancellationToken::new())
        .await
        .expect("the env dumper must run");
    assert_eq!(outcome.exit_code, Some(0));
    assert!(
        !outcome.stdout.text.contains("SAYA_HOST_PROBE_PLANTED"),
        "the planted name must be absent from the child's env: {:?}",
        outcome.stdout.text
    );
    assert!(
        !outcome.stdout.text.contains("hunter2-planted"),
        "the planted value must be absent from the child's env"
    );
}

#[tokio::test]
async fn an_explicitly_passed_var_reaches_the_child() {
    let dir = leak("pass");
    stage(&dir, "pass-probe");
    let config = config(
        &dir.display().to_string(),
        &[("SAYA_HOST_EXTRA", "extra-value")],
    );
    let outcome = HostCommand::new("pass-probe", ["env".to_owned()])
        .expect("shaped")
        .run(&config, None, &saya_agent::CancellationToken::new())
        .await
        .expect("the env dumper must run");
    assert_eq!(outcome.exit_code, Some(0));
    assert!(
        outcome.stdout.text.contains("SAYA_HOST_EXTRA=extra-value"),
        "an explicitly passed var must reach the child: {:?}",
        outcome.stdout.text
    );
}
