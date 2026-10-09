use super::*;

/// RED-first (E2-2): the cancellation-aware source variant does not exist
/// yet — this fails to compile until `execute_with_cancellation_and_source`
/// lands next to `execute_resolved_with_source`.
#[test]
fn cancellation_aware_source_variant_mirrors_the_plain_envelope() {
    let _pool = test_permit_lock();
    let mut fx = SaveRoundTripFixture::build();
    seed_five_rows(&db_path(&fx));
    set_max_rows(&mut fx, 2);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime builds");

    let execution = runtime
        .block_on(sql_operation::execute_with_cancellation_and_source(
            &fx.runtime,
            Some("analytics"),
            "SELECT * FROM t5 ORDER BY id",
            false,
            &CancellationToken::new(),
        ))
        .expect("the cancellation-aware source variant succeeds");
    let profile = fx
        .runtime
        .named_profile("analytics")
        .expect("fixture profile");
    let expected_identity = profile_identity("analytics", profile, &fx.runtime.cache_scope);
    assert_eq!(execution.binding.profile_name, "analytics");
    assert_eq!(
        execution.binding.profile_identity,
        expected_identity.as_str(),
        "the variant binds the SAME opaque profile identity",
    );
    assert_eq!(execution.binding.dialect, SqlDialect::Sqlite);
    assert_eq!(
        execution.binding.applied_row_cap, 2,
        "the variant binds the cap the query actually used",
    );
    assert_eq!(execution.result.rows.len(), 2);
    assert!(execution.result.truncated);
    assert!(execution.started_unix_ms <= execution.finished_unix_ms);

    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let stopped = runtime
        .block_on(sql_operation::execute_with_cancellation_and_source(
            &fx.runtime,
            Some("analytics"),
            "SELECT * FROM t5 ORDER BY id",
            false,
            &cancelled,
        ))
        .expect_err("a pre-cancelled variant stops before connector setup");
    assert_eq!(stopped.to_string(), "query cancelled");
}

/// Proof (a): after a direct SQL task completes through the REAL worker, the
/// captured evidence names the EXECUTED source — opaque identity (not None)
/// and the cap the query actually used — with nondecreasing timestamps.
#[test]
fn tui_worker_evidence_comes_from_the_executed_envelope() {
    let _pool = test_permit_lock();
    let mut fx = SaveRoundTripFixture::build();
    seed_five_rows(&db_path(&fx));
    set_max_rows(&mut fx, 2);
    let profile = fx
        .runtime
        .named_profile("analytics")
        .expect("fixture profile");
    let expected_identity = profile_identity("analytics", profile, &fx.runtime.cache_scope)
        .as_str()
        .to_owned();

    assert!(matches!(
        fx.dispatch_line("/connect analytics"),
        Dispatch::Handled
    ));
    let Dispatch::SqlTask(task) = fx.dispatch_line("/sql SELECT * FROM t5 ORDER BY id") else {
        panic!("/sql dispatches its worker task");
    };
    spawn_real_sql_task(&mut fx, task, &_pool);
    tick_until(
        &mut fx,
        |fx| fx.app.captured.is_some(),
        "the /sql worker never completed into the capture",
    );

    let capture = fx
        .app
        .captured
        .as_ref()
        .expect("a successful /sql is captured");
    assert_eq!(capture.result.rows.len(), 2);
    assert!(
        capture.result.truncated,
        "five rows through a cap of two report truncation",
    );
    let evidence = &capture.evidence;
    assert_eq!(
        evidence.connection_identity.as_deref(),
        Some(expected_identity.as_str()),
        "the evidence names the executed source identity, never None",
    );
    assert_eq!(
        evidence.max_rows, 2,
        "the evidence names the cap the query actually used",
    );
    assert_eq!(evidence.connection_label, "analytics");
    assert_eq!(evidence.dialect, SqlDialect::Sqlite);
    assert_eq!(evidence.source, EvidenceSource::DirectSql);
    assert!(
        evidence.started_unix_ms <= evidence.finished_unix_ms,
        "the envelope timestamps never run backwards",
    );
}

/// Proof (b) IMMUTABILITY: after spawning the task but BEFORE its completion
/// is consumed, the live runtime is mutated (row cap 2 -> 500, profile
/// endpoint repointed to a new identity) — the consumed evidence must still
/// describe the EXECUTED source.
#[test]
fn tui_worker_evidence_survives_a_runtime_mutation_before_consumption() {
    let _pool = test_permit_lock();
    let mut fx = SaveRoundTripFixture::build();
    seed_five_rows(&db_path(&fx));
    set_max_rows(&mut fx, 2);
    let profile = fx
        .runtime
        .named_profile("analytics")
        .expect("fixture profile");
    let expected_identity = profile_identity("analytics", profile, &fx.runtime.cache_scope)
        .as_str()
        .to_owned();

    assert!(matches!(
        fx.dispatch_line("/connect analytics"),
        Dispatch::Handled
    ));
    let Dispatch::SqlTask(task) = fx.dispatch_line("/sql SELECT id FROM t5 ORDER BY id") else {
        panic!("/sql dispatches its worker task");
    };
    spawn_real_sql_task(&mut fx, task, &_pool);

    // Mutate ONLY what the completion sees: the worker already owns its
    // runtime Arc, so this changes the consumption-time runtime alone.
    set_max_rows(&mut fx, 500);
    {
        let mut config = (*fx.runtime).clone();
        config.connections.profiles.insert(
            "analytics".to_string(),
            DatabaseProfile::Sqlite {
                path: "/nonexistent-mutated.sqlite3".to_string(),
                read_only: true,
            },
        );
        let fresh = Arc::new(config);
        fx.runtime = Arc::clone(&fresh);
        fx.app.runtime = fresh;
    }
    let mutated_identity = {
        let profile = fx
            .runtime
            .named_profile("analytics")
            .expect("mutated profile");
        profile_identity("analytics", profile, &fx.runtime.cache_scope)
            .as_str()
            .to_owned()
    };
    assert_ne!(
        mutated_identity, expected_identity,
        "the mutation really repoints the source identity",
    );

    tick_until(
        &mut fx,
        |fx| fx.app.captured.is_some(),
        "the /sql worker never completed into the capture",
    );
    let evidence = &fx
        .app
        .captured
        .as_ref()
        .expect("a successful /sql is captured")
        .evidence;
    assert_eq!(
        evidence.max_rows, 2,
        "the evidence keeps the EXECUTED cap, not the mutated 500",
    );
    assert_eq!(
        evidence.connection_identity.as_deref(),
        Some(expected_identity.as_str()),
        "the evidence keeps the EXECUTED identity, not the mutated endpoint",
    );
}

/// Proof (c): a read-only-rejected query through the REAL worker keeps the
/// old error behavior and captures nothing.
#[test]
fn tui_worker_read_only_rejection_keeps_error_behavior_and_captures_nothing() {
    use crate::interactive::tui::transcript::BlockKind;

    let _pool = test_permit_lock();
    let mut fx = SaveRoundTripFixture::build();
    set_max_rows(&mut fx, 2);
    assert!(matches!(
        fx.dispatch_line("/connect analytics"),
        Dispatch::Handled
    ));
    let sql = "DELETE FROM t";
    let Dispatch::SqlTask(task) = fx.dispatch_line(&format!("/sql {sql}")) else {
        panic!("/sql dispatches its worker task");
    };
    spawn_real_sql_task(&mut fx, task, &_pool);
    tick_until(
        &mut fx,
        |fx| fx.app.sql_task.is_none(),
        "the rejected worker never settled",
    );

    assert!(
        fx.app.captured.is_none(),
        "a rejected query captures nothing",
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime builds");
    let expected = runtime
        .block_on(sql_operation::execute(
            &fx.runtime,
            Some("analytics"),
            sql,
            false,
        ))
        .expect_err("direct SQL cannot write");
    let rendered = fx
        .app
        .transcript
        .blocks()
        .iter()
        .rev()
        .find(|block| block.kind == BlockKind::Error)
        .map(|block| block.text.clone())
        .expect("the rejection renders as an error block");
    assert_eq!(rendered, expected.to_string());
}
