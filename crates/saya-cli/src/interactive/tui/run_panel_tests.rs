//! The run panel's behaviors, driven through the seams the event loop uses:
//! the worker's channel (`run_worker::spawn`'s shape), `poll_run_panel` —
//! the drain the loop calls every tick — and the panel's cancel. A real
//! worker is spawned once, against a provider that cannot be reached, so the
//! "run is a worker task" claim is tested end to end rather than asserted.

#[cfg(test)]
#[path = "run_panel_support.rs"]
mod support;
#[cfg(test)]
#[path = "run_panel_worker.rs"]
mod worker;

use support::panel_app;

use super::run_panel::RunStepStatus;
use super::run_worker::{RunMsg, RunOutcome};
use saya_types::RunEvent;
use std::time::{Duration, Instant};

/// Red test 1: a run driven from the TUI renders its steps, and the step
/// list updates as steps complete — through the same drain the event loop
/// calls, from the approval ask that seeds the plan to the final lifecycle
/// event. The session's conversation must stay empty the whole time.
#[test]
fn a_run_driven_from_the_tui_renders_its_steps_and_updates_as_they_complete() {
    let (mut app, tx, _cancel) = panel_app("r-panel-steps", "survey the data");
    let (respond, answer) = tokio::sync::oneshot::channel();
    // The plan binds: the approval ask carries the step goals the panel seeds.
    tx.send(RunMsg::PlanApproval {
        view_text: "run approval — goal: survey the data".into(),
        steps: vec!["ask the data".into(), "write the note".into()],
        respond,
    })
    .unwrap();
    tx.send(RunMsg::Event(RunEvent::RunStarted)).unwrap();
    app.poll_run_panel(false);
    {
        let panel = app.run_panel.as_ref().unwrap();
        assert_eq!(panel.steps.len(), 2, "the plan's steps seeded the list");
        assert_eq!(panel.steps[0].goal, "ask the data");
        assert_eq!(panel.steps[0].status, RunStepStatus::Pending);
        assert_eq!(panel.status, "run started");
        assert!(
            app.transcript.blocks().is_empty(),
            "the run does not touch the conversation"
        );
    }

    // Step 1 runs and completes; step 2 starts. The panel advances with each
    // drain, exactly as the loop would show it.
    tx.send(RunMsg::Event(RunEvent::StepStarted { step: 0 }))
        .unwrap();
    app.poll_run_panel(false);
    assert_eq!(
        app.run_panel.as_ref().unwrap().steps[0].status,
        RunStepStatus::Running
    );
    assert_eq!(app.run_panel.as_ref().unwrap().status, "step 1 started");

    tx.send(RunMsg::Event(RunEvent::StepCompleted { step: 0 }))
        .unwrap();
    tx.send(RunMsg::Event(RunEvent::StepStarted { step: 1 }))
        .unwrap();
    app.poll_run_panel(false);
    let panel = app.run_panel.as_ref().unwrap();
    assert_eq!(panel.steps[0].status, RunStepStatus::Completed);
    assert!(
        panel.steps[0].elapsed.is_some(),
        "the completed step froze its elapsed"
    );
    assert!(
        panel.steps[0].started.is_none(),
        "no live clock lingers on a done step"
    );
    assert_eq!(panel.steps[1].status, RunStepStatus::Running);
    assert_eq!(panel.status, "step 2 started");
    // The modal the panel is showing is answerable — only an explicit yes
    // approves, and the drive's oneshot carries the decision.
    app.answer_plan_approval(true);
    assert!(
        answer.blocking_recv().unwrap(),
        "the modal's yes reaches the drive's decision"
    );

    // The run completes and the drive ends; the panel keeps the record.
    tx.send(RunMsg::Event(RunEvent::StepCompleted { step: 1 }))
        .unwrap();
    tx.send(RunMsg::Event(RunEvent::Completed)).unwrap();
    tx.send(RunMsg::Done(RunOutcome {
        code: 0,
        message: String::new(),
    }))
    .unwrap();
    app.poll_run_panel(false);
    let panel = app.run_panel.as_ref().unwrap();
    assert!(panel.terminated, "a terminal event marks the run over");
    assert_eq!(panel.status, "run completed");
    assert!(!panel.is_active(), "the worker is released once it ends");
    assert!(
        app.transcript.blocks().is_empty(),
        "still nothing in the conversation"
    );
}

/// Red test 2: the UI stays responsive while a run executes. A worker
/// behaves like the real one — an event, then quiet work, then the end — and
/// every `poll_run_panel` (the loop's per-tick drain) returns immediately
/// while the run is in flight.
#[test]
fn the_event_loop_is_not_blocked_while_a_run_executes() {
    let (mut app, tx, _cancel) = panel_app("r-panel-resp", "long job");
    std::thread::spawn(move || {
        tx.send(RunMsg::Event(RunEvent::StepStarted { step: 0 }))
            .unwrap();
        std::thread::sleep(Duration::from_millis(250));
        tx.send(RunMsg::Done(RunOutcome {
            code: 0,
            message: String::new(),
        }))
        .unwrap();
    });
    // The drain happens while the worker is still running: it must not wait
    // on the worker. The worker may not have sent its first event yet, so
    // poll until it lands — every poll in the loop must be prompt, which is
    // the responsiveness claim itself.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let t0 = Instant::now();
        app.poll_run_panel(false);
        assert!(
            t0.elapsed() < Duration::from_millis(100),
            "the per-tick drain must not block on the worker (took {:?})",
            t0.elapsed()
        );
        if app.run_panel.as_ref().unwrap().status == "step 1 started" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the worker's first event never landed"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    // The run is still in flight — nothing blocked, nothing finished yet.
    assert!(app.run_panel.as_ref().unwrap().is_active());

    // Give the worker time to finish, then one more prompt drain applies it.
    std::thread::sleep(Duration::from_millis(250));
    let t1 = Instant::now();
    app.poll_run_panel(false);
    assert!(
        t1.elapsed() < Duration::from_millis(100),
        "draining the end must be as prompt as draining progress"
    );
    assert!(
        !app.run_panel.as_ref().unwrap().is_active(),
        "the run's end was applied by a non-blocking poll"
    );
}
