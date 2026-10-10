//! W06b child-command environment battery (authored, not executed): the exact
//! six-variable allowlist, poisoned-command scrub, private dirs, no egress.

use std::{fs, path::Path};

use saya_agent::CancellationToken;
use saya_harness::coding::worktree::{
    ChildEnvironment, ChildEnvironmentError, ChildNetworkPolicy, OwnershipInput, confine_checkout,
    create_managed_checkout, verify_owned_checkout,
};
use saya_types::{JobId, ManagedCheckoutId};

use super::create_support::CreateCheckoutFixture;
use super::support;

/// The checkout under confinement in every test.
const A_ID: &str = "checkout-w06-a";
/// The exact variable allowlist; nothing else survives `env_clear`.
const ALLOWLIST: [&str; 6] = [
    "PATH",
    "HOME",
    "TMPDIR",
    "XDG_CONFIG_HOME",
    "LC_ALL",
    "LANG",
];
/// Variables the shell itself exports; not part of the environment handoff.
const SHELL_ADDED: [&str; 4] = ["PWD", "OLDPWD", "SHLVL", "_"];

/// One ownership input over the fixture for the given asking job.
fn ownership_input<'a>(
    fixture: &'a CreateCheckoutFixture,
    job_id: &'a JobId,
    checkout_id: &'a ManagedCheckoutId,
) -> OwnershipInput<'a> {
    OwnershipInput {
        store: &fixture.store,
        service: &fixture.service,
        repository: &fixture.repository,
        artifacts: &fixture.artifacts,
        job_id,
        checkout_id,
    }
}

/// The (name, value) pairs exported by a child's `export -p` output.
fn exported_vars(output: &str) -> Vec<(String, String)> {
    let mut vars = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim_start();
        let rest = if let Some(rest) = trimmed.strip_prefix("declare -x ") {
            rest
        } else if let Some(rest) = trimmed.strip_prefix("export ") {
            rest
        } else {
            continue;
        };
        let Some((name, value)) = rest.split_once('=') else {
            continue;
        };
        vars.push((name.to_string(), unquote(value)));
    }
    vars
}

/// Strips ONE matching outer quote pair (`"` from bash, `'` from dash).
fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() >= 2
        && (bytes[0] == b'"' || bytes[0] == b'\'')
        && bytes[bytes.len() - 1] == bytes[0]
    {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
}

/// A confined child sees exactly the six allowlisted variables, nothing else.
#[tokio::test]
async fn child_environment_is_exactly_the_allowlist() {
    let fixture = CreateCheckoutFixture::new("w06b-allowlist").await;
    let a = ManagedCheckoutId::parse(A_ID).unwrap();
    let token = CancellationToken::new();
    create_managed_checkout(&fixture.input(&a), &token)
        .await
        .unwrap();
    let input = ownership_input(&fixture, &fixture.job, &a);
    let verified = verify_owned_checkout(&input, &token).await.unwrap();
    let confined = confine_checkout(&verified, Path::new("/bin")).unwrap();
    let managed = fixture.service.managed_path(&a).unwrap();

    let output = confined
        .command(Path::new("/bin/sh"))
        .args(["-c", "export -p > env.txt"])
        .output()
        .unwrap();
    assert!(output.status.success(), "the env dump failed");

    let dumped = fs::read_to_string(format!("{}/env.txt", managed.as_str())).unwrap();
    let names: Vec<String> = exported_vars(&dumped)
        .into_iter()
        .map(|(name, _)| name)
        .filter(|name| !SHELL_ADDED.contains(&name.as_str()))
        .collect();
    for name in &names {
        assert!(
            ALLOWLIST.contains(&name.as_str()),
            "the child saw {name}: {names:?}"
        );
    }
    for name in ALLOWLIST {
        assert!(
            names.iter().any(|seen| seen == name),
            "missing {name}: {names:?}"
        );
    }
}

/// `apply` scrubs canaries from a poisoned command; HOME is under the tempdir.
#[test]
fn apply_clears_a_poisoned_command() {
    let root = support::TempRoot::new("w06b-poison-root");
    let environment = ChildEnvironment::prepare(root.path(), Path::new("/bin")).unwrap();
    let mut command = std::process::Command::new("/bin/sh");
    command
        .env("SAYA_CANARY_SECRET", "canary-secret-value")
        .env("OPENAI_API_KEY", "canary-key");
    environment.apply(&mut command);
    let output = command.args(["-c", "export -p"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("SAYA_CANARY_SECRET")
            && !stdout.contains("canary-secret-value")
            && !stdout.contains("OPENAI_API_KEY")
            && !stdout.contains("canary-key"),
        "a canary name or value leaked: {stdout}"
    );
    let home = exported_vars(&stdout)
        .into_iter()
        .find(|(name, _)| name == "HOME")
        .expect("HOME is exported")
        .1;
    assert!(
        home.ends_with(".saya/home") && home.contains(root.path().to_string_lossy().as_ref()),
        "the home value is {home}"
    );
}

/// Confined `$TMPDIR`/`$HOME` writes land inside `<checkout>/.saya/`.
#[tokio::test]
async fn child_tmp_and_home_live_inside_the_checkout() {
    let fixture = CreateCheckoutFixture::new("w06b-home-tmp").await;
    let a = ManagedCheckoutId::parse(A_ID).unwrap();
    let token = CancellationToken::new();
    create_managed_checkout(&fixture.input(&a), &token)
        .await
        .unwrap();
    let input = ownership_input(&fixture, &fixture.job, &a);
    let verified = verify_owned_checkout(&input, &token).await.unwrap();
    let confined = confine_checkout(&verified, Path::new("/bin")).unwrap();
    let managed = fixture.service.managed_path(&a).unwrap();

    let output = confined
        .command(Path::new("/bin/sh"))
        .args([
            "-c",
            "echo x > \"$TMPDIR/t.txt\" && echo y > \"$HOME/h.txt\"",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "the private-dir writes failed");

    let tmp_file = format!("{}/.saya/tmp/t.txt", managed.as_str());
    let home_file = format!("{}/.saya/home/h.txt", managed.as_str());
    assert_eq!(fs::read_to_string(&tmp_file).unwrap(), "x\n");
    assert_eq!(fs::read_to_string(&home_file).unwrap(), "y\n");
}

/// No loopback egress (EPERM evidence, listener silent); local write still runs.
#[tokio::test]
async fn child_command_has_no_transport_secrets_or_implicit_egress() {
    let fixture = CreateCheckoutFixture::new("w06b-egress").await;
    let a = ManagedCheckoutId::parse(A_ID).unwrap();
    let token = CancellationToken::new();
    create_managed_checkout(&fixture.input(&a), &token)
        .await
        .unwrap();
    let input = ownership_input(&fixture, &fixture.job, &a);
    let verified = verify_owned_checkout(&input, &token).await.unwrap();
    let confined = confine_checkout(&verified, Path::new("/bin")).unwrap();

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let connect_script = format!("exec 3<>/dev/tcp/127.0.0.1/{port} && printf ping >&3");
    let control = std::process::Command::new("/bin/bash")
        .args(["-c", &connect_script])
        .output()
        .unwrap();
    assert!(
        control.status.success(),
        "the unsandboxed control could not connect"
    );
    let mut attempts = 0;
    let (mut stream, _) = loop {
        match listener.accept() {
            Ok(pair) => break pair,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && attempts < 100 => {
                attempts += 1;
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(e) => panic!("the control connection never arrived: {e}"),
        }
    };
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .unwrap();
    let mut received = [0u8; 4];
    std::io::Read::read_exact(&mut stream, &mut received).unwrap();
    assert_eq!(&received, b"ping");
    let egress = confined
        .command(Path::new("/bin/bash"))
        .args(["-c", &connect_script])
        .output()
        .unwrap();
    assert!(
        !egress.status.success(),
        "the child connected out: {connect_script}"
    );
    if cfg!(target_os = "macos") {
        let stderr = String::from_utf8_lossy(&egress.stderr);
        assert!(
            stderr.contains("Operation not permitted"),
            "the macOS denial is missing: {stderr}"
        );
    }
    assert!(
        matches!(listener.accept(), Err(err) if err.kind() == std::io::ErrorKind::WouldBlock),
        "the listener received a connection"
    );

    let local = confined
        .command(Path::new("/bin/sh"))
        .args(["-c", "echo ok > w06b-ok.txt"])
        .output()
        .unwrap();
    assert!(local.status.success(), "the local write failed");
    let managed = fixture.service.managed_path(&a).unwrap();
    let ok_file = format!("{}/w06b-ok.txt", managed.as_str());
    assert_eq!(fs::read_to_string(&ok_file).unwrap(), "ok\n");
}

/// Typed refusals: a symlinked private root and a missing program dir.
#[test]
fn prepare_refuses_a_symlinked_private_dir() {
    let root = support::TempRoot::new("w06b-symlink-root");
    let target = support::TempRoot::new("w06b-symlink-target");
    std::os::unix::fs::symlink(target.path(), root.path().join(".saya")).unwrap();
    let prepared = ChildEnvironment::prepare(root.path(), Path::new("/bin"));
    assert!(matches!(prepared, Err(ChildEnvironmentError::PrivateDir)));
    let missing = ChildEnvironment::prepare(root.path(), Path::new("/nonexistent-w06b"));
    assert!(matches!(missing, Err(ChildEnvironmentError::ProgramDir)));
}

/// The only authorized network shape is no egress.
#[test]
fn network_policy_is_no_egress() {
    let root = support::TempRoot::new("w06b-policy-root");
    let environment = ChildEnvironment::prepare(root.path(), Path::new("/bin")).unwrap();
    assert_eq!(environment.network(), ChildNetworkPolicy::NoEgress);
}
