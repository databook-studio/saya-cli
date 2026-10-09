/// One table, one walk: every mutating slash command abandons the pending
/// clarification (the next begin mints a different question and the old
/// token is cancelled); every no-op or rejected command preserves it (the
/// next begin re-begins the SAME question, token live). The rows share one
/// real session and evolve its state in order; each row normalizes the
/// lifecycle to PendingClarification first.
#[test]
fn a_context_change_abandons_the_pending_question_and_a_noop_preserves_it() {
    let table: Vec<Row> = vec![
        // ---- ABANDON: each real mutation of the answering context ----
        (
            "/connect demo selects the profile",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Connect("demo".into()));
                assert_message(&action, label);
                assert_abandoned(h, &pending, label);
            }),
        ),
        (
            "/include demo adds the profile to the answering set",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Include("demo".into()));
                assert_message(&action, label);
                assert_abandoned(h, &pending, label);
            }),
        ),
        (
            "/exclude demo removes the profile from the answering set",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Exclude("demo".into()));
                assert_message(&action, label);
                assert_abandoned(h, &pending, label);
            }),
        ),
        (
            "/provider openai switches the provider",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Provider(Some("openai".into())));
                assert_message(&action, label);
                assert_abandoned(h, &pending, label);
            }),
        ),
        (
            "/model m2 switches the model",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Model(Some("m2".into())));
                assert_message(&action, label);
                assert_abandoned(h, &pending, label);
            }),
        ),
        (
            "/mode plan switches the task posture",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Mode(Some(AgentMode::Plan)));
                assert_message(&action, label);
                assert_abandoned(h, &pending, label);
            }),
        ),
        (
            "/privacy on flips data sharing",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Privacy(Some(true)));
                assert_message(&action, label);
                assert_abandoned(h, &pending, label);
            }),
        ),
        (
            "/approval read-only is a non-escalating mode change",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Approvals(Some(ApprovalPolicy::ReadOnly)));
                assert_message(&action, label);
                assert_abandoned(h, &pending, label);
            }),
        ),
        (
            "/tasks clear empties the task list",
            Box::new(|h, label| {
                let pending = h.pend(label);
                h.state.task_list = SessionTaskList::new(vec![
                    SessionTask::with_note(
                        "context-key task",
                        TaskStatus::InProgress,
                        None::<&str>,
                    )
                    .expect("a valid task"),
                ])
                .expect("a valid task list");
                let action = h.run(SlashCommand::Tasks(Some("clear".into())));
                assert_message(&action, label);
                assert_abandoned(h, &pending, label);
            }),
        ),
        (
            "/clear wipes the conversation history",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Clear);
                assert_message(&action, label);
                assert_abandoned(h, &pending, label);
            }),
        ),
        // ---- PRESERVE: a command that changes nothing ----
        (
            "/help is a usage query",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Help(None));
                assert_message(&action, label);
                assert_preserved(h, &pending, label);
            }),
        ),
        (
            "/model with no argument only reports",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Model(None));
                assert_message(&action, label);
                assert_preserved(h, &pending, label);
            }),
        ),
        (
            "/provider with no argument only reports",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Provider(None));
                assert_message(&action, label);
                assert_preserved(h, &pending, label);
            }),
        ),
        (
            "/mode with no argument only reports",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Mode(None));
                assert_message(&action, label);
                assert_preserved(h, &pending, label);
            }),
        ),
        (
            "/usage is a no-op diagnostic",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Usage);
                assert_message(&action, label);
                assert_preserved(h, &pending, label);
            }),
        ),
        (
            "setting the model to its current value changes nothing",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Model(Some("m2".into())));
                assert_message(&action, label);
                assert_preserved(h, &pending, label);
            }),
        ),
        (
            "a rejected change (unknown provider) errors without mutating",
            Box::new(|h, label| {
                let pending = h.pend(label);
                let action = h.run(SlashCommand::Provider(Some("nosuch-provider".into())));
                let SessionAction::Error(message) = action else {
                    panic!("{label}: an unsupported provider is refused, got {action:?}");
                };
                assert!(
                    message.contains("Unsupported provider"),
                    "{label}: the refusal names the problem: {message}"
                );
                assert_preserved(h, &pending, label);
            }),
        ),
    ];
    let mut harness = Harness::new("table");
    for (label, step) in &table {
        step(&mut harness, label);
    }
    let root = harness.sessions_root.clone();
    drop(harness);
    let _ = std::fs::remove_dir_all(&root);
}
