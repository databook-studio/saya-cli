//! The `/dev/null` contract of the Linux confinement, run through the real
//! confinement: POSIX gives an asynchronous list (`cmd &`) its standard input
//! from `/dev/null`, and `>/dev/null` opens the device with `O_TRUNC`. Both
//! must work inside the sandbox; every other device stays denied — the rule
//! is on the one file, never on `/dev`.

use super::{Proven, proven_shell};
use crate::runner::ShellCall;

/// The startup-probe convention of this battery: skip with the printed
/// reason where the probe did not prove the sandbox — unless
/// `SAYA_REQUIRE_LANDLOCK=1` asks for the Linux checks, where a host that
/// cannot prove Landlock must fail the test rather than pass it silently.
fn required(proven: Option<Proven>, tag: &str) -> Option<Proven> {
    if proven.is_none() && std::env::var("SAYA_REQUIRE_LANDLOCK").as_deref() == Ok("1") {
        panic!(
            "`{tag}`: the startup probe did not prove the Linux sandbox and \
             SAYA_REQUIRE_LANDLOCK=1 forbids skipping"
        );
    }
    proven
}

/// A background command and a `/dev/null` redirection both succeed: the
/// device is the POSIX stdin of an asynchronous list and the target of
/// `O_TRUNC` redirections.
#[tokio::test]
async fn background_commands_and_dev_null_redirection_are_allowed() {
    let Some(proven) = required(proven_shell("dev-null-allowed"), "dev-null-allowed") else {
        return;
    };

    let background = proven
        .shell
        .run(ShellCall::new("sleep 0 & wait"))
        .await
        .expect("the shell must run");
    assert_eq!(
        background.exit_code,
        Some(0),
        "a background command gets its stdin from /dev/null: {:?}",
        background.stderr.text
    );

    let redirected = proven
        .shell
        .run(ShellCall::new("echo x >/dev/null"))
        .await
        .expect("the shell must run");
    assert_eq!(
        redirected.exit_code,
        Some(0),
        "a >/dev/null redirection must open the device: {:?}",
        redirected.stderr.text
    );
}

/// The grant is the file, not the directory: reading `/dev/zero` and listing
/// `/dev` are both refused with EACCES evidence.
#[tokio::test]
async fn every_other_device_and_the_dev_directory_stay_denied() {
    let Some(proven) = required(proven_shell("dev-null-denied"), "dev-null-denied") else {
        return;
    };

    let zero = proven
        .shell
        .run(ShellCall::new("cat /dev/zero | head -c1"))
        .await
        .expect("the shell must run");
    assert!(
        zero.stdout.text.is_empty(),
        "no byte may come from /dev/zero: {:?}",
        zero.stdout.text
    );
    assert!(
        zero.stderr.text.contains("Permission denied"),
        "the /dev/zero open must carry EACCES evidence: {:?}",
        zero.stderr.text
    );

    let dev = proven
        .shell
        .run(ShellCall::new("ls /dev"))
        .await
        .expect("the shell must run");
    assert_ne!(
        dev.exit_code,
        Some(0),
        "the /dev directory must stay unreachable: {:?}",
        dev.stdout.text
    );
    assert!(
        dev.stderr.text.contains("Permission denied"),
        "the /dev listing must carry EACCES evidence: {:?}",
        dev.stderr.text
    );
}
