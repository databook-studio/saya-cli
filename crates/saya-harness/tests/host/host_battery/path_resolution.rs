//! Program resolution and its typed refusals: only the built PATH is
//! searched, shaped and traversal names refuse at every layer, an absent
//! name names where it looked, and a directory entry is not executable.

use std::fs;

use super::{config, leak, stage};

use saya_harness::host::{HostCommand, HostError};

#[tokio::test]
async fn resolution_uses_the_built_path_not_the_parent_s() {
    // A program on the parent's PATH (a temp dir prepended to it) but
    // absent from the built PATH must not resolve.
    let dir = leak("parent-path");
    stage(&dir, "only-on-parent-path");
    let old_path = std::env::var("PATH").unwrap_or_default();
    let joined = format!("{}:{}", dir.display(), old_path);
    unsafe { std::env::set_var("PATH", &joined) };
    let empty = leak("built-path-empty");
    let config = config(&empty.display().to_string(), &[]);
    let error = HostCommand::new("only-on-parent-path", Vec::<String>::new())
        .expect("shaped")
        .run(&config, None, &saya_agent::CancellationToken::new())
        .await
        .expect_err("a name off the built PATH must refuse");
    unsafe { std::env::set_var("PATH", &old_path) };
    assert!(
        matches!(&error, HostError::NotOnPath { program, .. } if program == "only-on-parent-path"),
        "the refusal must be the typed not-on-PATH variant: {error}"
    );
}

#[tokio::test]
async fn path_shaped_and_traversal_names_refuse_at_every_layer() {
    let dir = leak("shapes");
    stage(&dir, "probe");
    let searched = dir.display().to_string();
    for name in [
        "./probe",
        "/bin/probe",
        "../probe",
        "sub/dir",
        "probe/",
        ".",
        "..",
    ] {
        let parse = HostCommand::new(name, Vec::<String>::new());
        assert!(
            matches!(&parse, Err(HostError::NameNotBare { .. })),
            "{name} must refuse at construction"
        );
        let config = config(&searched, &[]);
        let error = config
            .resolve(name)
            .expect_err(&format!("{name} must refuse at resolution"));
        assert!(
            matches!(&error, HostError::NameNotBare { .. }),
            "{name} must refuse at resolution: {error}"
        );
    }
}

#[tokio::test]
async fn a_name_not_on_the_built_path_refuses_with_the_path_it_searched() {
    let empty = leak("no-such");
    let config = config(&empty.display().to_string(), &[]);
    let error = HostCommand::new("no-such-program", Vec::<String>::new())
        .expect("shaped")
        .run(&config, None, &saya_agent::CancellationToken::new())
        .await
        .expect_err("a name off the PATH must refuse");
    let HostError::NotOnPath { program, searched } = &error else {
        panic!("the refusal must be the typed not-on-PATH variant: {error}");
    };
    assert_eq!(program, "no-such-program");
    assert!(
        searched.contains(&empty.display().to_string()),
        "the error must name where it looked: {error}"
    );
}

#[tokio::test]
async fn a_directory_on_the_path_does_not_resolve() {
    let dir = leak("dir-entry");
    fs::create_dir_all(dir.join("a-dir")).expect("a dir entry must plant");
    let config = config(&dir.display().to_string(), &[]);
    let error = HostCommand::new("a-dir", Vec::<String>::new())
        .expect("shaped")
        .run(&config, None, &saya_agent::CancellationToken::new())
        .await
        .expect_err("a directory must refuse, not exec");
    assert!(
        matches!(&error, HostError::NotOnPath { .. }),
        "a directory on the PATH must refuse as not resolvable: {error}"
    );
}
