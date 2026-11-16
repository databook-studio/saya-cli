use super::support::Fixture;
use clap::Parser;
use saya_agent::CancellationToken;
use saya_cli::{Cli, CodingApplication, Command, dispatch_coding, load_with_sources};
use saya_config::BudgetLimit;
use std::{collections::BTreeMap, fs};

fn finite(value: u64) -> BudgetLimit {
    BudgetLimit::Finite(std::num::NonZeroU64::new(value).unwrap())
}

#[tokio::test]
async fn coding_cli_bounds_reach_runtime_and_read_only_doctor() {
    let fixture = Fixture::new("coding-cli-bounds").await;
    std::fs::write(
        fixture.root.join("config.toml"),
        "[limits.job]\nlogical_requests = 45\n\
         transport_attempts = 180\njob_tokens = 600000\n\
         elapsed_seconds = 1200\n",
    )
    .unwrap();
    let bare = Cli::try_parse_from([
        "saya",
        "--coding-max-logical-model-requests",
        "30",
        "--coding-max-physical-transport-attempts",
        "90",
        "--coding-max-job-tokens",
        "400000",
        "--coding-max-elapsed-seconds",
        "600",
    ])
    .expect("REPL launch accepts native coding bounds without a subcommand");
    assert!(bare.command.is_none());
    let cli = Cli::try_parse_from([
        "saya",
        "job",
        "doctor",
        "--coding-max-logical-model-requests",
        "30",
        "--coding-max-physical-transport-attempts",
        "90",
        "--coding-max-job-tokens",
        "400000",
        "--coding-max-elapsed-seconds",
        "600",
    ])
    .expect("real CLI accepts explicit native coding bounds");
    let command = match cli.command {
        Some(Command::Job { command, .. }) => command,
        other => panic!("expected the parsed job doctor command, got {other:?}"),
    };
    let runtime = load_with_sources(
        &cli.options,
        &fixture.repository,
        &fixture.root,
        BTreeMap::new(),
    )
    .expect("CLI bounds resolve through the normal config layers");
    assert_eq!(
        runtime
            .resolved
            .provenance
            .get("limits.job.logical_requests")
            .map(|origin| origin.source.clone()),
        Some(saya_config::SettingSource::Cli {
            flag: "--coding-max-logical-model-requests".into()
        }),
        "a coding flag is attributed to the key it overrides"
    );
    let application = CodingApplication::open(
        runtime,
        fixture.store.clone(),
        &fixture.repository,
        &fixture.root.join("coding"),
        &fixture.git,
        &fixture.program_dir,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let report = dispatch_coding(&application, command).await.unwrap();
    assert_eq!(report["limits"]["max_logical_model_requests"], 30);
    assert_eq!(report["limits"]["max_physical_transport_attempts"], 90);
    assert_eq!(report["limits"]["max_elapsed_seconds"], 600);
    assert_eq!(report["token_ceilings"]["job_total"]["limit"], 400_000);
    assert_eq!(report["token_ceilings"]["job_total"]["enforced"], true);

    let omitted = Cli::try_parse_from(["saya", "job", "doctor"]).unwrap();
    let doctor = match omitted.command {
        Some(Command::Job { command, .. }) => command,
        other => panic!("expected the parsed job doctor command, got {other:?}"),
    };
    let unchanged = load_with_sources(
        &omitted.options,
        &fixture.repository,
        &fixture.root,
        BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(unchanged.resolved.limits.job.logical_requests, finite(45));
    assert_eq!(
        unchanged.resolved.limits.job.transport_attempts,
        finite(180)
    );
    assert_eq!(unchanged.resolved.limits.job.job_tokens, finite(600_000));
    assert_eq!(unchanged.resolved.limits.job.elapsed_seconds, finite(1_200));
    // The same numbers reach the loop the job is created under and the
    // read-only report the operator reads them back from.
    let application = CodingApplication::open(
        unchanged,
        fixture.store.clone(),
        &fixture.repository,
        &fixture.root.join("coding"),
        &fixture.git,
        &fixture.program_dir,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let report = dispatch_coding(&application, doctor).await.unwrap();
    assert_eq!(report["budget_mode"], "bounded");
    assert_eq!(report["limits"]["max_logical_model_requests"], 45);
    assert_eq!(report["limits"]["max_physical_transport_attempts"], 180);
    assert_eq!(report["limits"]["max_elapsed_seconds"], 1_200);
    assert_eq!(report["token_ceilings"]["job_total"]["limit"], 600_000);

    let invalid = Cli::try_parse_from([
        "saya",
        "--coding-max-logical-model-requests",
        "31",
        "--coding-max-physical-transport-attempts",
        "30",
        "job",
        "doctor",
    ])
    .unwrap();
    assert!(
        load_with_sources(
            &invalid.options,
            &fixture.repository,
            &fixture.root,
            BTreeMap::new(),
        )
        .is_err(),
        "CLI overrides still pass through the existing cross-limit validation"
    );

    let no_config = load_with_sources(
        &cli.options,
        &fixture.repository,
        &fixture.root.join("no-coding-config"),
        BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(
        no_config.resolved.limits.job.logical_requests,
        finite(30),
        "budget flags apply to a config that never mentions coding"
    );
}

#[tokio::test]
async fn uncapped_mode_rejects_global_numeric_budget_overrides() {
    let fixture = Fixture::new("uncapped-cli-conflict").await;
    fs::write(
        fixture.root.join("config.toml"),
        "[limits.job]\nbudget_mode = \"uncapped\"\n",
    )
    .unwrap();
    for (flag, value, setting) in [
        (
            "--coding-max-logical-model-requests",
            "1",
            "limits.job.logical_requests",
        ),
        (
            "--coding-max-physical-transport-attempts",
            "240",
            "limits.job.transport_attempts",
        ),
        ("--coding-max-job-tokens", "1", "limits.job.job_tokens"),
        (
            "--coding-max-elapsed-seconds",
            "1",
            "limits.job.elapsed_seconds",
        ),
    ] {
        let cli = Cli::try_parse_from(["saya", "job", "doctor", flag, value]).unwrap();
        let error = load_with_sources(
            &cli.options,
            &fixture.repository,
            &fixture.root,
            BTreeMap::new(),
        )
        .expect_err("an explicit CLI cap cannot be combined with uncapped mode");
        let message = error.to_string();
        assert!(
            message.contains("uncapped") && message.contains(setting),
            "wrong conflict for {flag}: {message}"
        );
    }
}
