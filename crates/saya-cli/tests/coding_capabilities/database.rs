use super::{Fixture, Provider, results, sqlite_fixture};
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingApplication, CodingAuthorization, CodingError, CodingJobRequest};
use serde_json::json;

async fn fixture(
    provider: &Provider,
    label: &str,
    sharing: bool,
) -> (Fixture, sqlx::SqlitePool, std::path::PathBuf) {
    let mut fixture = Fixture::with_endpoint(label, Some(&provider.endpoint)).await;
    let selected = sqlite_fixture::select_scratch_sqlite(&mut fixture, sharing).await;
    (fixture, selected.pool, selected.elsewhere)
}

#[tokio::test]
async fn selected_database_reads_and_schema_reject_writes_and_unselected_profiles() {
    let provider = Provider::database();
    let (fixture, pool, elsewhere) = fixture(&provider, "db-capability", true).await;
    let request: CodingJobRequest =
        serde_json::from_value::<CodingJobRequest>(json!({"job_id":"db-capability",
        "task":"Read only the approved database", "write_paths":["new-a.txt","new-b.txt"],
        "programs":[], "database_profiles":["scratch","zzmissing"],
        "tools_allow":["helpers","read"]}))
        .expect("explicit database scope must be supported");
    let proposal = fixture.application.propose(&request).await.unwrap();
    let approval = CodingAuthorization::scoped(&proposal).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(30), job.run(&NoopEventSink))
        .await
        .unwrap()
        .unwrap();
    assert!(
        results(&provider, "root-schema")
            .iter()
            .any(|r| r.contains("saya_sentinel"))
    );
    assert!(
        results(&provider, "root-read")
            .iter()
            .any(|r| r.contains("approved-row"))
    );
    assert!(
        results(&provider, "root-unselected")
            .iter()
            .any(|r| r.contains("error") && !r.contains("approved-row"))
    );
    // A helper gets the same read-only database tools the root does.
    assert!(
        results(&provider, "child-schema")
            .iter()
            .any(|r| r.contains("saya_sentinel"))
    );
    assert!(
        results(&provider, "child-read")
            .iter()
            .any(|r| r.contains("approved-row"))
    );
    assert!(
        results(&provider, "child-sql-write")
            .iter()
            .any(|r| r.contains("error") && r.contains("read-only"))
    );
    assert!(
        results(&provider, "child-sql-unselected")
            .iter()
            .any(|r| r.contains("error") && !r.contains("approved-row"))
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM saya_sentinel")
            .fetch_one(&pool)
            .await
            .unwrap(),
        2
    );
    for tool in ["child-read", "root-read"] {
        let reads = results(&provider, tool);
        let read: serde_json::Value = serde_json::from_str(reads.last().unwrap()).unwrap();
        assert_eq!(read["rows"].as_array().unwrap().len(), 1);
        assert_eq!(read["truncated"], true);
    }
    assert!(!elsewhere.exists(), "unselected profile must never connect");
    assert!(
        !fixture.root.join("missing.sqlite").exists(),
        "selected coding schema reads must not create a SQLite file"
    );
    assert!(
        results(&provider, "root-missing-schema")
            .iter()
            .any(|r| r.contains("error"))
    );
    pool.close().await;
}

#[tokio::test]
async fn privacy_and_legacy_requests_do_not_grant_row_authority() {
    for selected in [true, false] {
        let provider = Provider::database();
        let (fixture, pool, elsewhere) = fixture(&provider, "db-privacy", false).await;
        let mut value = json!({"job_id":"db-privacy","task":"Read approved data only",
            "write_paths":["new-a.txt","new-b.txt"],"programs":[],
            "tools_allow":["helpers","read"]});
        if selected {
            value["database_profiles"] = json!(["scratch"]);
        }
        let request: CodingJobRequest = serde_json::from_value::<CodingJobRequest>(value).unwrap();
        let approval =
            CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap())
                .unwrap();
        let job = fixture
            .application
            .create(request, &approval, CancellationToken::new())
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(30), job.run(&NoopEventSink))
            .await
            .unwrap()
            .unwrap();
        let read = results(&provider, "child-read");
        assert!(read.iter().all(|r| !r.contains("approved-row")));
        if selected {
            assert!(read.iter().any(|r| r.contains("error")), "{read:?}");
            assert!(
                results(&provider, "root-read")
                    .iter()
                    .any(|r| r.contains("error"))
            );
            for body in provider.bodies.lock().unwrap().iter() {
                let body: serde_json::Value = serde_json::from_str(body).unwrap();
                assert!(
                    body["tools"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|tool| tool["function"]["name"] != "bounded_sql_query"),
                    "row tools must be hidden when sharing is disabled"
                );
            }
            // The schema tool is not a row tool: it stays advertised.
            assert!(
                results(&provider, "child-schema")
                    .iter()
                    .any(|r| r.contains("saya_sentinel"))
            );
        } else {
            for body in provider.bodies.lock().unwrap().iter() {
                let body: serde_json::Value = serde_json::from_str(body).unwrap();
                assert!(
                    body["tools"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|tool| !matches!(
                            tool["function"]["name"].as_str(),
                            Some("bounded_sql_query" | "schema_discovery")
                        ))
                );
            }
            // A durable request with neither field still resumes without a grant.
            let path = fixture.root.join("coding/db-privacy/request.json");
            let mut saved: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            saved.as_object_mut().unwrap().remove("database_profiles");
            saved.as_object_mut().unwrap().remove("database_identities");
            std::fs::write(path, serde_json::to_vec(&saved).unwrap()).unwrap();
            assert_eq!(
                fixture
                    .application
                    .propose_resume(job.job_id())
                    .await
                    .unwrap()
                    .request()["database_profiles"],
                json!({})
            );
        }
        assert!(!elsewhere.exists());
        pool.close().await;
    }
}

#[tokio::test]
async fn resume_refuses_changed_identity_under_an_approved_profile_name() {
    let provider = Provider::database();
    let (mut fixture, pool, elsewhere) = fixture(&provider, "db-drift", true).await;
    let request: CodingJobRequest = serde_json::from_value::<CodingJobRequest>(json!({"job_id":"db-drift",
        "task":"Read approved data only","write_paths":[],"programs":[],"database_profiles":["scratch"]})).unwrap();
    let proposed = fixture.application.propose(&request).await.unwrap();
    assert!(
        proposed.request()["database_profiles"]["scratch"]
            .as_str()
            .is_some()
    );
    assert_eq!(
        proposed.request()["database_origin"]["project_provided"],
        true
    );
    assert!(
        proposed.request()["database_origin"]["path"]
            .as_str()
            .unwrap()
            .ends_with(".saya/connections.toml")
    );
    let approval = CodingAuthorization::scoped(&proposed).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    fixture.runtime.connections = saya_config::ConnectionsFile::from_toml(&format!(
        "[profiles.scratch]\ntype='sqlite'\npath='{}'\n",
        elsewhere.display()
    ))
    .unwrap();
    let changed = CodingApplication::open(
        fixture.runtime.clone(),
        fixture.store.clone(),
        &fixture.repository,
        &fixture.root.join("coding"),
        &fixture.git,
        &fixture.program_dir,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert!(matches!(
        changed.propose_resume(job.job_id()).await,
        Err(CodingError::ApprovalRequired)
    ));
    assert!(matches!(
        changed
            .resume(job.job_id(), &approval, CancellationToken::new())
            .await,
        Err(CodingError::ApprovalRequired)
    ));
    assert!(!elsewhere.exists());
    pool.close().await;
}
