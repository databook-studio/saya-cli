/// A detached `/export --refresh` must not write its file when its result
/// arrives late: detach drops the receiver, so the completion — the only
/// place the export is written — never runs. The undetached control proves
/// the file can be written at all, so the refusal is the detach, not the
/// fixture.
#[test]
fn detached_refresh_export_writes_no_file() {
    fn export_task(path: &Path) -> SqlTask {
        SqlTask::new(
            Some("local".to_string()),
            "SELECT 1 AS one".to_string(),
            Followup::Export {
                request: ExportRequest {
                    mode: Some(ExportMode::Refresh),
                    overwrite: false,
                    path: path.to_string_lossy().into_owned(),
                },
            },
        )
    }

    let dir = temp_dir("refresh-export");
    let path = dir.join("out.csv");

    // The control: without a detach the completion writes the file.
    {
        let mut app = idle_app();
        let mut state = SessionState::new("test", None, "model");
        let store = session_store(&dir);
        let (tx, rx) = mpsc::channel();
        app.sql_task = Some((
            rx,
            export_task(&path),
            Instant::now(),
            saya_agent::CancellationToken::new(),
        ));
        tx.send(TerminalEvent::QueryResult {
            result: sample_result(),
        })
        .expect("the late result arrives");
        tick_workers(&mut app, &store, &mut state);
        assert!(
            path.exists(),
            "control: the undetached refresh writes the file"
        );
    }
    let _ = std::fs::remove_file(&path);

    // The pinned behaviour: detach first, result arrives late, no file.
    {
        let mut app = idle_app();
        let mut state = SessionState::new("test", None, "model");
        let store = session_store(&dir);
        let (tx, rx) = mpsc::channel();
        app.sql_task = Some((
            rx,
            export_task(&path),
            Instant::now(),
            saya_agent::CancellationToken::new(),
        ));
        app.request.started = Some(Instant::now());
        app.request.activity = Some("query".into());
        app.detach_sql_task();
        // The worker's result lands on the dropped receiver — the send the
        // real worker makes (`let _ = tx.send(event)`) fails exactly like this.
        tx.send(TerminalEvent::QueryResult {
            result: sample_result(),
        })
        .expect_err("the receiver was dropped by the detach");
        tick_workers(&mut app, &store, &mut state);
        assert!(
            !path.exists(),
            "a detached refresh must never write its export file"
        );
        let system = last_block(&app.transcript, BlockKind::System).expect("detach is said");
        assert!(
            system.contains("Cancellation requested; query detached"),
            "the detach message is present: {system}"
        );
        assert!(
            last_block(&app.transcript, BlockKind::System)
                .is_none_or(|text| !text.contains("Exported")),
            "no export success is reported after detach"
        );
    }
}
