//! Red tests for the split bar: the bottom bar names the database and model
//! (and, conditionally, the mode and tasks), never the posture — the top
//! context line keeps that unchanged. See `TASK.md`'s "Decided design" for
//! the exact segment order and shedding hierarchy these pin.

use super::super::context_line::context_words_for_test;
use super::bar_words_for_test;
use crate::interactive::session_prompt::StatusView;
use crate::interactive::tui::ui_snapshot_tests::{empty_app, fixed_status, render_buffer};

/// The owner's example session from `TASK.md`'s "Why": `docker_postgres`,
/// `openai_compatible/glm-5.2`, approval `ask`, mode `build`, a bound
/// workspace, a composed host lane, sharing on.
fn example_view() -> StatusView {
    StatusView {
        profile: "docker_postgres".into(),
        included: Vec::new(),
        provider: "openai_compatible".into(),
        model: "glm-5.2".into(),
        approval_mode: "ask".into(),
        agent_mode: "build".into(),
        workspace_root: Some("/Users/subodhsharma/Projects/saya-cli".into()),
        sharing_on: true,
        host_composed: true,
        denied_programs: Vec::new(),
        task_summary: None,
    }
}

/// Every combination of posture conditions the top line may or may not show
/// — the space `posture_tokens_never_reach_the_bar` and `nothing_leaves_the_screen`
/// both sweep.
fn posture_combos() -> Vec<StatusView> {
    let mut combos = Vec::new();
    for sharing_on in [true, false] {
        for host_composed in [true, false] {
            for approval_mode in ["read-only", "ask", "never", "bypass"] {
                for denied_programs in [Vec::new(), vec!["curl".to_string()]] {
                    for workspace_root in [Some("/workspace/root".to_string()), None] {
                        combos.push(StatusView {
                            profile: "docker_postgres".into(),
                            included: Vec::new(),
                            provider: "openai_compatible".into(),
                            model: "glm-5.2".into(),
                            approval_mode: approval_mode.to_string(),
                            agent_mode: "build".into(),
                            workspace_root: workspace_root.clone(),
                            sharing_on,
                            host_composed,
                            denied_programs: denied_programs.clone(),
                            task_summary: None,
                        });
                    }
                }
            }
        }
    }
    combos
}

/// The session from the owner's example: the bar's words are exactly the
/// database and the model, plus the right-aligned hint — none of the
/// posture this session also carries (approval `ask`, a bound workspace, a
/// composed host, sharing on).
#[test]
fn the_bar_names_the_database_and_model_only() {
    let words = bar_words_for_test(&example_view(), "? for help", 200);
    assert!(
        words.starts_with("docker_postgres · glm-5.2"),
        "the bar must lead with the database, then the model: {words:?}"
    );
    assert!(
        words.ends_with("? for help"),
        "the hint must end the bar: {words:?}"
    );
}

/// Across every posture combination, the bar carries none of the removed
/// tokens, nor the provider — they stayed on the top line.
#[test]
fn posture_tokens_never_reach_the_bar() {
    for view in posture_combos() {
        let words = bar_words_for_test(&view, "? for help", 200);
        for removed in [
            "approval:",
            "ws:",
            "host:",
            "deny:",
            "sharing:",
            "openai_compatible",
        ] {
            assert!(
                !words.contains(removed),
                "the bar must never carry {removed:?}: {words:?}"
            );
        }
    }
}

/// Across the same combinations, every condition the bar dropped is still
/// named on the top line at a wide width — nothing the bar stopped saying
/// left the screen entirely.
#[test]
fn nothing_leaves_the_screen() {
    for view in posture_combos() {
        let top = context_words_for_test(&view, 500);
        if view.sharing_on {
            assert!(top.contains("Data sharing on"), "{top:?}");
        }
        if view.approval_mode != "read-only" {
            assert!(
                top.contains(&format!("Approval: {}", view.approval_mode)),
                "{top:?}"
            );
        }
        if view.host_composed {
            assert!(top.contains("Host commands unsandboxed"), "{top:?}");
        }
        if !view.denied_programs.is_empty() {
            assert!(top.contains("Denied:"), "{top:?}");
        }
        match view.workspace_root.as_deref() {
            Some(root) => assert!(top.contains(root), "{top:?}"),
            None => assert!(top.contains("No workspace bound"), "{top:?}"),
        }
    }
}

/// Included profiles collapse to a count on the database segment, and are
/// silent when there are none.
#[test]
fn included_profiles_collapse_to_a_count() {
    let mut view = example_view();
    view.profile = "car_1".into();
    view.included = vec!["car_2".into(), "car_3".into(), "car_4".into()];
    let words = bar_words_for_test(&view, "? for help", 200);
    assert!(
        words.contains("car_1 +3"),
        "three included profiles must collapse to +3: {words:?}"
    );

    view.included = Vec::new();
    let words = bar_words_for_test(&view, "? for help", 200);
    assert!(
        !words.contains('+'),
        "no included profiles must leave no +: {words:?}"
    );
}

/// The mode segment paints only away from the default: absent at `build`,
/// present at `plan`.
#[test]
fn mode_shows_only_when_not_build() {
    let mut view = example_view();
    view.agent_mode = "build".into();
    let words = bar_words_for_test(&view, "? for help", 200);
    assert!(
        !words.contains("mode:") && !words.contains("build"),
        "the default mode must show no mode segment at all: {words:?}"
    );

    view.agent_mode = "plan".into();
    let words = bar_words_for_test(&view, "? for help", 200);
    assert!(
        words.contains("plan") && !words.contains("mode:"),
        "plan must show the bare mode word, with no mode: prefix: {words:?}"
    );
}

/// The tasks segment paints only while a list exists.
#[test]
fn tasks_show_while_a_list_exists() {
    let mut view = example_view();
    view.task_summary = Some("tasks: 2/4".into());
    let words = bar_words_for_test(&view, "? for help", 200);
    assert!(words.contains("tasks: 2/4"), "{words:?}");

    view.task_summary = None;
    let words = bar_words_for_test(&view, "? for help", 200);
    assert!(!words.contains("tasks:"), "{words:?}");
}

/// The hint is right-aligned: at a real frame width, the painted row's last
/// cell is the hint's last character — through the real `ui::draw`, the same
/// buffer-rendering approach `status_cell_tests` uses.
#[test]
fn the_hint_is_right_aligned() {
    let app = empty_app();
    let buffer = render_buffer(&app, &fixed_status(), 120, 10);
    let row = buffer
        .lines()
        .find(|line| line.contains("? for help"))
        .expect("the bar row paints the hint");
    assert!(
        row.trim_end_matches('"').ends_with("? for help"),
        "the hint's last cell must sit on the bar's last column:\n{row}"
    );
}

/// A narrow row sheds the model, mode, and tasks before it sheds the
/// database name or the hint; narrower still, only the database remains.
#[test]
fn a_narrow_bar_sheds_from_the_end() {
    let mut view = example_view();
    view.agent_mode = "plan".into();
    view.task_summary = Some("tasks: 2/4".into());

    // 28 cells fit the 16-cell name, a 1-cell gap, and the 11-cell hint;
    // nothing else.
    let words = bar_words_for_test(&view, "? for help", 30);
    assert!(words.starts_with("docker_postgres"), "{words:?}");
    assert!(!words.contains("glm-5.2"), "the model must shed: {words:?}");
    assert!(!words.contains("plan"), "the mode must shed: {words:?}");
    assert!(!words.contains("tasks:"), "tasks must shed: {words:?}");
    assert!(
        words.contains("? for help"),
        "the hint must survive: {words:?}"
    );

    // Below the name-plus-hint floor, the hint sheds too; the name never
    // does.
    let words = bar_words_for_test(&view, "? for help", 20);
    assert!(words.contains("docker_postgres"), "{words:?}");
    assert!(
        !words.contains("? for help"),
        "the hint must shed before the database name does: {words:?}"
    );
}
