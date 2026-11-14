//! What the root advertises of its shell family, and how `[tools] allow`
//! governs it: the start is advertised with the shell, the two calls that reach
//! a background run join the list only once this job has started one, and the
//! family stands or falls as the one name `shell`.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::sync::Arc;

use saya_agent::{ApprovalPolicy, ToolExecutor};
use serde_json::json;

use super::direct::RootTools;
use super::direct_tests::compose_root_data;
use super::direct_tools::DirectCodingTools;
use super::parent::CodingTools;
use super::root_shell::RootShell;
use super::root_shell_background_tests::{reader, shell_over};
use super::root_shell_tests::fixture::TempCheckout;
use crate::agent::tools::child::tests_root_compose_fixture as fixture;

/// The root's composed toolset over a working checkout, with `shell` attached.
async fn root_with_shell(
    fixture: &fixture::Fixture,
    shell: Option<RootShell>,
) -> DirectCodingTools {
    let workspace = compose_root_data(fixture, saya_config::ResolvedTools::default(), None).await;
    let coordinator = CodingTools {
        job: Arc::clone(&fixture.delegate_job),
        databases: None,
        denials: crate::agent::tools::coding::owner_denial::DenialTally::default(),
    };
    let decider = workspace.approval;
    DirectCodingTools::new(
        coordinator,
        workspace.tools,
        workspace.definitions,
        decider,
        shell,
        &saya_config::HelperAgents::default(),
        None,
        None,
        None,
        String::new(),
        saya_config::ResolvedTools::default(),
    )
}

/// The root advertises the start with its shell and neither reader before a
/// run exists, and neither of the three without a shell at all: a root with no
/// shell has no run to read.
#[tokio::test]
async fn the_root_advertises_the_background_family_with_its_shell() {
    let fixture = fixture::Fixture::new("root-background-advertise", &["tracked.txt"]).await;
    let checkout = TempCheckout::new("root-background-advertise");
    let bare = root_with_shell(&fixture, None).await;
    for name in ["shell", "shell_output", "shell_kill"] {
        assert!(!bare.advertises(name), "no shell means no {name}");
    }

    let Some(shell) = shell_over(&checkout, ApprovalPolicy::Bypass, None) else {
        return;
    };
    let composed = root_with_shell(&fixture, Some(shell)).await;
    assert!(composed.advertises("shell"), "the start is advertised");
    for name in ["shell", "shell_output", "shell_kill"] {
        assert!(
            DirectCodingTools::is_shell(name),
            "{name} routes to the root shell"
        );
    }
    for name in ["shell_output", "shell_kill"] {
        assert!(
            !composed.advertises(name),
            "{name} is not advertised before a background run exists"
        );
    }
    assert!(
        composed.approves_coordinator(&reader("shell_output")),
        "the reader is the definition the root approves once it is advertised"
    );
}

/// The two readers join the tool list only once this job has started a
/// background run: before that they are not advertised and a call to either is
/// refused as an unknown tool, and after it they are — once, and then the list
/// stays stable for the rest of the run.
#[tokio::test]
async fn the_readers_join_the_tool_list_only_after_a_background_start() {
    let fixture = fixture::Fixture::new("root-background-gate", &["tracked.txt"]).await;
    let checkout = TempCheckout::new("root-background-gate");
    let Some(shell) = shell_over(&checkout, ApprovalPolicy::Bypass, None) else {
        return;
    };
    let root = RootTools::WorkingCheckout(Box::new(root_with_shell(&fixture, Some(shell)).await));
    let listed = |root: &RootTools| {
        root.definitions()
            .into_iter()
            .map(|definition| definition.name)
            .collect::<Vec<_>>()
    };

    let before = listed(&root);
    assert!(before.contains(&"shell".to_owned()), "{before:?}");
    for name in ["shell_output", "shell_kill"] {
        assert!(!before.contains(&name.to_owned()), "{before:?}");
        assert!(
            matches!(
                root.execute(name, json!({"id": 1})).await,
                Err(saya_agent::ToolError::UnsupportedTool)
            ),
            "a call to {name} before a run exists is an unknown tool"
        );
    }
    // A foreground call is not a background start: the list does not move.
    root.execute("shell", json!({"command": "true", "sandbox": true}))
        .await
        .expect("a foreground call runs");
    assert_eq!(
        listed(&root),
        before,
        "a foreground call advertises nothing"
    );

    let started = root
        .execute(
            "shell",
            json!({"command": "sleep 30", "run_in_background": true, "sandbox": true}),
        )
        .await
        .expect("a background start runs");
    let id = started["id"]
        .as_u64()
        .expect("the start answers with an id");
    let after = listed(&root);
    for name in ["shell_output", "shell_kill"] {
        assert!(after.contains(&name.to_owned()), "{after:?}");
    }
    // The list only grows, and the two readers join the shell family they
    // belong to: every name advertised before keeps the position it had, and
    // the notebook edit — the one tool this list appends last — stays last.
    let mut expected = before.clone();
    let shell = expected
        .iter()
        .position(|name| name == "shell")
        .expect("the start is advertised");
    for name in ["shell_kill", "shell_output"] {
        expected.insert(shell + 1, name.to_owned());
    }
    assert_eq!(
        after, expected,
        "the readers join the shell family and nothing else moves"
    );
    assert_eq!(listed(&root), after, "and it stays stable afterwards");

    let killed = root
        .execute("shell_kill", json!({"id": id}))
        .await
        .expect("the kill reaches the run the start made");
    assert_eq!(killed["status"], "killed");
    assert_eq!(listed(&root), after, "the list stays stable to the end");
}

/// The family is one name in `[tools] allow`. A list that keeps `shell` keeps
/// the start and both readers, and a list without it keeps none of the three:
/// a start whose reader the list removed would be a promise nothing could
/// keep, and a reader whose start was removed would reach nothing.
#[tokio::test]
async fn the_shell_family_stands_or_falls_as_one_name() {
    let fixture = fixture::Fixture::new("root-background-narrowing", &["tracked.txt"]).await;
    let checkout = TempCheckout::new("root-background-narrowing");
    let Some(shell) = shell_over(&checkout, ApprovalPolicy::Bypass, None) else {
        return;
    };
    let mut composed = root_with_shell(&fixture, Some(shell)).await;
    let stated = |words: &[&str]| {
        saya_config::ResolvedTools::from_stated(Some(
            &words
                .iter()
                .map(|word| (*word).to_string())
                .collect::<Vec<_>>(),
        ))
        .expect("a valid list")
    };
    let composed_names = |composed: &DirectCodingTools| {
        composed
            .defs
            .iter()
            .map(|definition| definition.name.clone())
            .collect::<Vec<_>>()
    };

    crate::agent::tools::narrow(&mut composed.defs, &stated(&["shell"]));
    for name in ["shell", "shell_output", "shell_kill"] {
        assert!(
            composed_names(&composed).contains(&name.to_owned()),
            "`shell` keeps {name}"
        );
    }
    assert!(composed.advertises("shell"));

    crate::agent::tools::narrow(&mut composed.defs, &stated(&["read"]));
    for name in ["shell", "shell_output", "shell_kill"] {
        assert!(
            !composed_names(&composed).contains(&name.to_owned()),
            "a list without `shell` drops {name}"
        );
        assert!(
            !composed.advertises(name),
            "a list without `shell` unadvertises {name}"
        );
    }
}
