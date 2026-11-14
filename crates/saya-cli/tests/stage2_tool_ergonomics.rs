//! Stage 2 tool-ergonomics contracts at public configuration and child-tool
//! boundaries. Runtime lifecycle coverage is added only after these feature
//! reds make the supported surfaces reachable.

use saya_config::{ConfigFile, ConnectionsFile, ResolutionInput, resolve};
#[cfg(target_os = "macos")]
use serde_json::Value;
#[cfg(unix)]
#[path = "coding_application/extensions.rs"]
mod extensions;
#[cfg(unix)]
#[path = "coding_capabilities/provider.rs"]
#[allow(dead_code)]
mod provider;
#[cfg(unix)]
#[allow(dead_code)] // the Linux counterpart uses only part of the fixture
#[path = "coding_application/sqlite_fixture.rs"]
mod sqlite_fixture;
#[cfg(unix)]
#[allow(dead_code)] // the macOS journey uses this fixture; Linux pins the refusal
#[path = "stage2_tool_ergonomics/support.rs"]
mod stage2_support;
#[cfg(unix)]
#[path = "coding_application/support.rs"]
#[allow(dead_code)]
mod support;
#[cfg(unix)]
impl provider::Provider {
    fn stage2() -> Self {
        Self::script(36)
    }
}
#[test]
fn trusted_automatic_lsp_selector_resolves_an_existing_lsp_profile() {
    // This is deliberately raw trusted configuration: model arguments and
    // repository configuration must never select an automatic diagnostic
    // process. The profile itself remains subject to the real runner grant at
    // native child dispatch.
    let config = ConfigFile::from_toml(
        "[job]\nautomatic_lsp_profile = 'rust-lsp'\n\n[job.extensions.rust-lsp]\nprotocol = 'lsp'\nprogram = 'bash'\nargv = ['-c', 'exit 0']\nlanguage = 'rust'\n",
    )
    .expect("the trusted automatic selector is an accepted coding setting");
    let resolved = resolve(ResolutionInput::new(ConnectionsFile::default()).with_user(config))
        .expect("the selected LSP profile resolves with ordinary profile validation");

    assert!(
        resolved.coding.extensions.contains_key("rust-lsp"),
        "the automatic selector names an existing trusted LSP profile"
    );
}
// The journey runs a confined LSP extension process, which only macOS proves.
// On Linux the extension cannot be started and the run fails closed; the
// Linux counterpart at the end of this file pins that refusal.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn native_stage2_journey_delivers_batched_reads_ranges_edits_and_receipts_without_apply() {
    use saya_agent::{CancellationToken, NoopEventSink};
    use saya_cli::{CodingAuthorization, CodingJobRequest};
    use saya_types::{BudgetDimension, JobId};
    use sha2::{Digest, Sha256};
    use std::{process::Command, time::Duration};
    let provider = provider::Provider::stage2();
    let profile = extensions::extension_config(&extensions::lsp_server(false, false, false), true);
    let mut fixture =
        support::Fixture::with_extensions("stage2-ergonomics", &provider.endpoint, &profile).await;
    let selected = sqlite_fixture::select_scratch_sqlite(&mut fixture, true).await;
    for name in ["read-a.txt", "read-b.txt", "read-c.txt", "read-d.txt"] {
        std::fs::write(fixture.repository.join(name), format!("{name}\n")).unwrap();
    }
    std::fs::write(
        fixture.repository.join("large.txt"),
        (1..=1_000)
            .map(|line| format!("line-{line:04} {}\n", "x".repeat(80)))
            .collect::<String>(),
    )
    .unwrap();
    let baseline = b"alpha-token\nbeta-token\n";
    std::fs::write(fixture.repository.join("tracked.txt"), baseline).unwrap();
    let output = Command::new(&fixture.git)
        .args(["add", "tracked.txt"])
        .current_dir(&fixture.repository)
        .output()
        .unwrap();
    assert!(output.status.success());
    let output = Command::new(&fixture.git)
        .args(["commit", "--quiet", "-m", "stage2 baseline"])
        .current_dir(&fixture.repository)
        .output()
        .unwrap();
    assert!(output.status.success());
    let request = CodingJobRequest::new(
        JobId::parse("stage2-ergonomics").unwrap(),
        // "helper" is the request's own evidence for the lean tool set's
        // optional `helpers` tool; this journey scripts a real delegation.
        "exercise the native stage2 tool journey with a helper".into(),
        vec!["tracked.txt".into(), "large.txt".into()],
    )
    .unwrap()
    .with_database_profiles(vec!["scratch".into()])
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    let sink = NoopEventSink;
    let running = job.run(&sink);
    tokio::pin!(running);
    let outcome = match tokio::time::timeout(Duration::from_secs(45), &mut running).await {
        Ok(outcome) => outcome,
        Err(_) => {
            job.cancel();
            let settled = tokio::time::timeout(Duration::from_secs(15), &mut running).await;
            panic!("stage2 job exceeded 45 seconds; cancellation settlement={settled:?}");
        }
    };
    assert!(
        outcome.is_ok(),
        "stage2 native journey must settle: outcome={outcome:?}; diagnostics={:?}; known_commits={:?}",
        std::fs::read_to_string(
            fixture
                .root
                .join("coding")
                .join(job.job_id().as_str())
                .join("diagnostics-summary.json"),
        ),
        std::fs::read_dir(fixture.root.join("coding").join(job.job_id().as_str()))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("known-commits-"))
            .map(std::fs::read_to_string)
            .collect::<Vec<_>>(),
    );
    assert_eq!(
        std::fs::read(fixture.repository.join("tracked.txt")).unwrap(),
        baseline
    );
    let checkouts = fixture
        .store
        .list_managed_checkouts(job.job_id(), None, 8)
        .await
        .unwrap();
    let (working, candidate) = stage2_support::direct_checkouts(&checkouts.items);
    let candidate_id = candidate.ownership.managed_checkout_id().clone();
    assert_eq!(
        std::fs::read(
            std::path::Path::new(working.ownership.managed_path().as_str()).join("tracked.txt")
        )
        .unwrap(),
        b"beta-token\ngamma-token\n"
    );
    assert_eq!(
        std::fs::read(
            std::path::Path::new(candidate.ownership.managed_path().as_str()).join("tracked.txt")
        )
        .unwrap(),
        b"beta-token\ngamma-token\n"
    );
    let records = fixture
        .store
        .list_coding_budget_reservations(job.job_id(), None, 100)
        .await
        .unwrap();
    let commands = records
        .items
        .iter()
        .filter(|record| {
            record
                .reservation
                .charges()
                .iter()
                .any(|charge| charge.dimension() == BudgetDimension::CommandSlots)
        })
        .collect::<Vec<_>>();
    // No typed command ran: `shell` is the command tool, and it keeps no
    // command receipt and reserves no command slot.
    assert!(
        commands.is_empty(),
        "a journey that runs no typed command reserves no command slot"
    );
    assert_eq!(
        fixture
            .store
            .coding_recovery_status(job.job_id())
            .await
            .unwrap()
            .unresolved_reservations,
        0
    );
    let (history, body_count) = {
        let bodies = provider.bodies.lock().unwrap();
        (stage2_support::parse_history(&bodies), bodies.len())
    };
    let results = &history.results;
    let calls = &history.calls;
    let ranged = &results["stage2-range"];
    let large = std::fs::read(fixture.repository.join("large.txt")).unwrap();
    let expected_range = (900..=901)
        .map(|line| format!("{line:>6}→line-{line:04} {}\n", "x".repeat(80)))
        .collect::<String>();
    assert_eq!(ranged["content"], expected_range);
    assert_eq!(ranged["start_line"], 900);
    assert_eq!(ranged["line_count"], 2);
    assert_eq!(ranged["total_lines"], 1_000);
    assert_eq!(ranged["lines_returned"], 2);
    assert_eq!(ranged["selected_range_complete"], true);
    assert_eq!(ranged["truncated"], true);
    assert_eq!(ranged["size"], large.len() as u64);
    assert_eq!(ranged["digest"], format!("{:x}", Sha256::digest(&large)));
    let tracked = &results["stage2-tracked-read"];
    assert_eq!(
        tracked["content"],
        "     1→alpha-token\n     2→beta-token\n"
    );
    assert_eq!(tracked["digest"], format!("{:x}", Sha256::digest(baseline)));
    let edited = &results["stage2-multi-edit"];
    let final_bytes = b"beta-token\ngamma-token\n";
    assert_eq!(
        edited["digest"],
        format!("{:x}", Sha256::digest(final_bytes))
    );
    assert_eq!(edited["size"], final_bytes.len() as u64);
    assert_eq!(edited["post_edit_diagnostics"]["status"], "complete");
    assert_eq!(edited["post_edit_diagnostics"]["version"], 1);
    assert_eq!(
        edited["post_edit_diagnostics"]["document_sha256"],
        edited["digest"]
    );
    assert_eq!(edited["post_edit_diagnostics"]["edits_applied"], false);
    assert_eq!(
        edited["post_edit_diagnostics"]["diagnostics"][0]["message"],
        "fixture diagnostic"
    );
    assert_eq!(
        calls["stage2-multi-edit"].1["edits"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        calls["stage2-multi-edit"].1["expected_digest"], tracked["digest"],
        "an edit of an existing file carries the digest the read proved"
    );
    assert_eq!(calls["stage2-schema"].1["connection"], "scratch");
    assert_eq!(calls["stage2-select"].1["connection"], "scratch");
    assert!(
        results["stage2-schema"]
            .to_string()
            .contains("saya_sentinel")
    );
    stage2_support::assert_database(&results["stage2-select"], &selected).await;
    let extension_path = fixture
        .root
        .join("coding")
        .join(job.job_id().as_str())
        .join(format!(
            "extensions-{}.jsonl",
            working.ownership.managed_checkout_id().as_str()
        ));
    let extension_receipts = std::fs::read_to_string(extension_path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(extension_receipts.len(), 2);
    assert_eq!(extension_receipts[0]["fact"]["phase"], "spawned");
    assert_eq!(extension_receipts[1]["fact"]["phase"], "settled");
    assert_eq!(extension_receipts[1]["fact"]["joined"], true);
    let known_commit_path = fixture
        .root
        .join("coding")
        .join(job.job_id().as_str())
        .join(format!(
            "known-commits-{}.jsonl",
            working.ownership.managed_checkout_id().as_str()
        ));
    let known_commits = std::fs::read_to_string(known_commit_path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert!(known_commits.iter().any(|receipt| {
        receipt["operation"] == "edit"
            && receipt["path"] == "tracked.txt"
            && receipt["committed_sha256"] == edited["digest"]
            && receipt["size"] == final_bytes.len() as u64
    }));
    // The direct root has no `coding_review` and no prepare tool: the owner
    // prepares the candidate from W at the end of the run, and the receipt it
    // leaves behind is what the review binds to. The review itself is asserted
    // after the resume boundary, through the operator path the `coding review`
    // command drives.
    let candidate_digest = candidate_id.as_str().trim_start_matches("candidate-");
    assert_eq!(candidate_digest.len(), 64, "{candidate_id}");
    assert!(
        fixture
            .root
            .join("coding")
            .join(job.job_id().as_str())
            .join(format!("prepared-candidate-{candidate_digest}.json"))
            .exists(),
        "the prepared receipt is durable for this generation"
    );
    let resumed = stage2_support::assert_resume_boundary(
        &fixture,
        job.job_id(),
        &approval,
        &checkouts.items,
        &records.items,
        &provider,
        body_count,
    )
    .await;
    stage2_support::assert_reviewed_prepared_candidate(&resumed, &candidate_id).await;
    selected.pool.close().await;
}

// The Linux counterpart: no confined LSP process can start, so the first
// committed edit fails the run closed. The edit's known-commit receipt is
// already durable, and no extension process was spawned.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn stage2_extension_journey_fails_closed_without_proven_confinement() {
    use saya_agent::{CancellationToken, NoopEventSink};
    use saya_cli::{CodingAuthorization, CodingJobRequest};
    use saya_types::JobId;
    use std::{process::Command, time::Duration};

    let provider = provider::Provider::stage2();
    let profile = extensions::extension_config(&extensions::lsp_server(false, false, false), true);
    let mut fixture =
        support::Fixture::with_extensions("stage2-host-lane", &provider.endpoint, &profile).await;
    let _selected = sqlite_fixture::select_scratch_sqlite(&mut fixture, true).await;
    for name in ["read-a.txt", "read-b.txt", "read-c.txt", "read-d.txt"] {
        std::fs::write(fixture.repository.join(name), format!("{name}\n")).unwrap();
    }
    std::fs::write(
        fixture.repository.join("large.txt"),
        (1..=1_000)
            .map(|line| format!("line-{line:04} {}\n", "x".repeat(80)))
            .collect::<String>(),
    )
    .unwrap();
    std::fs::write(
        fixture.repository.join("tracked.txt"),
        b"alpha-token\nbeta-token\n",
    )
    .unwrap();
    let output = Command::new(&fixture.git)
        .args(["add", "tracked.txt"])
        .current_dir(&fixture.repository)
        .output()
        .unwrap();
    assert!(output.status.success());
    let output = Command::new(&fixture.git)
        .args(["commit", "--quiet", "-m", "stage2 baseline"])
        .current_dir(&fixture.repository)
        .output()
        .unwrap();
    assert!(output.status.success());
    let request = CodingJobRequest::new(
        JobId::parse("stage2-host-lane").unwrap(),
        "exercise the native stage2 tool journey with a helper".into(),
        vec!["tracked.txt".into(), "large.txt".into()],
    )
    .unwrap()
    .with_database_profiles(vec!["scratch".into()])
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(45), job.run(&NoopEventSink))
        .await
        .expect("the job settles");
    assert!(
        outcome.is_err(),
        "a confined extension cannot start on this host: {outcome:?}"
    );
    let records = fixture.root.join("coding").join(job.job_id().as_str());
    let extensions: Vec<String> = std::fs::read_dir(&records)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("extensions-"))
        .collect();
    assert!(
        extensions.is_empty(),
        "no extension process was spawned: {extensions:?}"
    );
}
