//! The cwd contract: the child runs with the bound root as its working
//! directory, even when the program lives elsewhere — observed through the
//! child's own `pwd` output and the file it writes.

use std::{fs, os::unix::fs::PermissionsExt as _, time::Duration};

use super::leak;

use saya_harness::host::{HostCommand, HostConfig};

#[tokio::test]
async fn the_child_runs_with_the_bound_root_as_its_cwd() {
    // The prompt's `cwd: pinned to <root>` line is a fact only because
    // the executor applies it: the child must observe the bound root as
    // its own working directory, even when the parent runs elsewhere.
    // The assertion reads the child's `pwd` output and the file it
    // writes — never the prompt string, which states the same line
    // under today's bug.
    let root = leak("cwd-root");
    let elsewhere = leak("cwd-elsewhere");
    let probe = elsewhere.join("probe.sh");
    fs::write(
        &probe,
        "#!/bin/sh\n/bin/pwd\n/usr/bin/touch child-wrote-here\n",
    )
    .expect("the cwd probe must plant");
    let mut perms = fs::metadata(&probe).expect("staged").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&probe, perms).expect("exec bit must set");
    let config = HostConfig::new(
        elsewhere.display().to_string(),
        root.clone(),
        Duration::from_secs(600),
    )
    .expect("shaped");
    let outcome = HostCommand::new("probe.sh", Vec::<String>::new())
        .expect("shaped")
        .run(&config, None, &saya_agent::CancellationToken::new())
        .await
        .expect("the cwd probe must run");
    assert_eq!(
        outcome.exit_code,
        Some(0),
        "stderr {:?}",
        outcome.stderr.text
    );
    let observed = std::path::PathBuf::from(outcome.stdout.text.trim());
    let canonical =
        |path: &std::path::Path| fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    assert_eq!(
        canonical(&observed),
        canonical(&root),
        "the child's own working directory is the bound root"
    );
    assert!(
        root.join("child-wrote-here").exists(),
        "the child's relative write landed in the bound root"
    );
    assert!(
        !elsewhere.join("child-wrote-here").exists(),
        "the child's relative write did not land beside the program"
    );
}
