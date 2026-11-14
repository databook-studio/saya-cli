#[test]
fn the_text_shape_prints_ready_and_one_line_per_check() {
    let checks = vec![
        TaskCheck {
            name: "provider".into(),
            status: CheckStatus::Ok,
            detail: "openai, model gpt-4o".into(),
            hint: None,
        },
        TaskCheck {
            name: "endpoint".into(),
            status: CheckStatus::Warn,
            detail: "configured endpoint: none".into(),
            hint: None,
        },
    ];
    let readiness = TaskReadiness {
        task: "ai",
        ready: true,
        checks,
    };
    let rendered = render_event(&event_from(&readiness), RenderFormat::Text);
    assert_eq!(
        rendered.stdout,
        "task ai: ready\n\
         ✓ provider: openai, model gpt-4o\n\
         ! endpoint: configured endpoint: none\n"
    );
}

#[test]
fn the_text_shape_marks_a_not_ready_task() {
    let checks = vec![TaskCheck {
        name: "profile".into(),
        status: CheckStatus::Fail,
        detail: "no connection profile is selected".into(),
        hint: None,
    }];
    let readiness = TaskReadiness {
        task: "query",
        ready: false,
        checks,
    };
    let rendered = render_event(&event_from(&readiness), RenderFormat::Text);
    assert_eq!(
        rendered.stdout,
        "task query: NOT ready\n\
         ✗ profile: no connection profile is selected\n"
    );
}
