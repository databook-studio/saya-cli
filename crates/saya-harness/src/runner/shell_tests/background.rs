//! Tests 16-24: background runs. A call started with `background: true`
//! returns its id before the command has finished, its output is readable
//! while it runs and after it ends, and its whole process group dies when it
//! is killed, when the job is cancelled, and when the shell itself goes away.
//! The number of runs one shell may hold is a hard cap, never a queue, and a
//! background run's unread output stays inside the same ring cap a foreground
//! run's capture does.

use std::{
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use saya_agent::CancellationToken;

use super::proven_shell;
use crate::runner::{MAX_BACKGROUND_RUNS, OUTPUT_CAP_BYTES, RunStatus, RunnerError, ShellCall};

/// A background call: the shape the root's `run_in_background` resolves to.
fn background(command: impl Into<String>) -> ShellCall {
    ShellCall::new(command).with_background(true)
}

/// A `sleep` that outlives the test, unique to it: the seconds keep two tests
/// in this binary from probing the same process, and the process id keeps a
/// stale process from an earlier run out of the probe.
fn sleeper(seconds: u32) -> String {
    format!("sleep {seconds}.{:04}", std::process::id() % 10_000)
}

/// Whether any process matches `marker` right now.
fn running(marker: &str) -> bool {
    Command::new("pgrep")
        .arg("-f")
        .arg(marker)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|output| !output.stdout.is_empty())
}

/// Polls until `marker` is gone or `deadline` passes, yielding to the runtime
/// between probes: the cancellation watcher runs on this runtime, so a
/// blocking poll would never let it kill anything.
async fn gone(marker: &str, deadline: Duration) -> bool {
    let until = Instant::now() + deadline;
    loop {
        if !running(marker) {
            return true;
        }
        if Instant::now() >= until {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Polls until `marker` is running or `deadline` passes, yielding to the
/// runtime between probes. A start returns its id as soon as the child is
/// spawned, and the marker reaches a process's command line only once that
/// child has exec'd — so a probe taken the instant the id comes back can miss
/// the very process it is asking about. Waiting on the condition is what
/// makes the liveness assertions deterministic; a group that is genuinely
/// missing still fails, `deadline` later.
async fn appears(marker: &str, deadline: Duration) -> bool {
    let until = Instant::now() + deadline;
    loop {
        if running(marker) {
            return true;
        }
        if Instant::now() >= until {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// How long a start may take to show up as a process: generous next to the
/// sub-tens-of-milliseconds an `exec` costs, and bounded so a group that never
/// appears fails the test rather than hanging it.
const APPEAR: Duration = Duration::from_secs(5);

/// How long a killed group may take to disappear: the kill is issued and the
/// child reaped before the call returns, and the grace covers a descendant
/// still waiting for its new parent to collect it.
const DISAPPEAR: Duration = Duration::from_secs(5);

/// (16) A background `sleep 5; echo late` returns an id immediately — the
/// start does not wait for the command — and the line it printed is there to
/// read once the run ends.
#[tokio::test]
async fn a_background_run_returns_an_id_at_once_and_its_output_is_readable() {
    let Some(proven) = proven_shell("background-read") else {
        return;
    };
    let started = Instant::now();
    let id = proven
        .shell
        .start_background(background("sleep 5; echo late"))
        .await
        .expect("a background call starts");
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "the start must not wait for the run; took {elapsed:?}"
    );

    let early = proven
        .shell
        .read_background(id, Duration::ZERO)
        .await
        .expect("the run this shell started is known to it");
    assert_eq!(early.status, RunStatus::Running, "the run is still going");
    assert_eq!(early.stdout.text, "", "nothing has been printed yet");

    let late = proven
        .shell
        .read_background(id, Duration::from_secs(8))
        .await
        .expect("the run this shell started is known to it");
    assert_eq!(late.status, RunStatus::Exited { exit_code: Some(0) });
    assert_eq!(late.stdout.text, "late\n");
}

/// (17) A read returns the output written since the last read, not the whole
/// window again: the second read of a run that printed twice returns only the
/// second line.
#[tokio::test]
async fn a_read_returns_only_the_output_since_the_last_read() {
    let Some(proven) = proven_shell("background-incremental") else {
        return;
    };
    let id = proven
        .shell
        .start_background(background("echo one; sleep 3; echo two"))
        .await
        .expect("a background call starts");
    let first = proven
        .shell
        .read_background(id, Duration::from_secs(1))
        .await
        .expect("the run is known");
    assert_eq!(first.status, RunStatus::Running, "three seconds is not up");
    assert_eq!(first.stdout.text, "one\n");
    let second = proven
        .shell
        .read_background(id, Duration::from_secs(8))
        .await
        .expect("the run is known");
    assert_eq!(
        second.stdout.text, "two\n",
        "the second read returns only what is new"
    );
    assert_eq!(second.status, RunStatus::Exited { exit_code: Some(0) });
}

/// (18) `shell_kill` kills the run's whole process group and reaps it: the
/// shell's background job is exactly the process a single-child kill misses.
#[tokio::test]
async fn killing_a_background_run_reaps_its_whole_process_group() {
    let Some(proven) = proven_shell("background-kill") else {
        return;
    };
    let marker = sleeper(601);
    let id = proven
        .shell
        .start_background(background(format!("{marker} & wait")))
        .await
        .expect("a background call starts");
    assert!(
        appears(&marker, APPEAR).await,
        "the detached job must be alive before the kill"
    );
    let killed = proven
        .shell
        .kill_background(id)
        .await
        .expect("the run is known");
    assert_eq!(killed.status, RunStatus::Killed);
    assert!(
        gone(&marker, DISAPPEAR).await,
        "no process from the killed group may survive"
    );
}

/// (19) The cap is a refusal, not a queue: the run past it is refused with the
/// cap's own number, and the refused start leaves no process behind.
#[tokio::test]
async fn starting_more_than_the_cap_is_refused_and_never_queued() {
    let Some(proven) = proven_shell("background-cap") else {
        return;
    };
    let mut ids = Vec::new();
    for _ in 0..MAX_BACKGROUND_RUNS {
        ids.push(
            proven
                .shell
                .start_background(background("sleep 30"))
                .await
                .expect("a start under the cap runs"),
        );
    }
    let refused = proven.shell.start_background(background("sleep 30")).await;
    assert!(
        matches!(
            refused,
            Err(RunnerError::BackgroundLimitReached { limit }) if limit == MAX_BACKGROUND_RUNS
        ),
        "the run past the cap is refused with the cap's number"
    );
    assert_eq!(
        proven.shell.background_runs(),
        MAX_BACKGROUND_RUNS,
        "a refused start adds no run"
    );
    for id in ids {
        proven
            .shell
            .kill_background(id)
            .await
            .expect("the run is known");
    }
    assert_eq!(
        proven.shell.background_runs(),
        0,
        "a killed run releases its slot"
    );
}

/// (20) Cancelling the job kills every live background group, and the run
/// reports the cancellation rather than a quiet exit.
#[tokio::test]
async fn cancelling_the_job_kills_every_live_background_group() {
    let Some(proven) = proven_shell("background-cancel") else {
        return;
    };
    let cancellation = CancellationToken::new();
    let proven = proven.with_cancellation(cancellation.clone());
    let first = sleeper(602);
    let second = sleeper(603);
    let first_id = proven
        .shell
        .start_background(background(format!("{first} & wait")))
        .await
        .expect("a background call starts");
    let second_id = proven
        .shell
        .start_background(background(format!("{second} & wait")))
        .await
        .expect("a second background call starts");
    assert!(
        appears(&first, APPEAR).await && appears(&second, APPEAR).await,
        "both detached jobs are alive before the cancellation"
    );

    cancellation.cancel();
    assert!(
        gone(&first, DISAPPEAR).await && gone(&second, DISAPPEAR).await,
        "the job's cancellation must kill every live group"
    );
    for id in [first_id, second_id] {
        let report = proven
            .shell
            .read_background(id, Duration::from_secs(5))
            .await
            .expect("the run is known");
        assert_eq!(report.status, RunStatus::Cancelled);
    }
}

/// (21) A shell that goes away takes its background runs with it: the job's
/// tools drop when the job ends, and no group outlives them.
#[tokio::test]
async fn a_shell_that_ends_leaves_no_background_group() {
    let Some(proven) = proven_shell("background-drop") else {
        return;
    };
    let marker = sleeper(604);
    proven
        .shell
        .start_background(background(format!("{marker} & wait")))
        .await
        .expect("a background call starts");
    assert!(appears(&marker, APPEAR).await, "the detached job is alive");
    let shell = proven.shell;
    drop(shell);
    assert!(
        gone(&marker, DISAPPEAR).await,
        "the job's end must kill and reap every live group"
    );
}

/// A background run's unread output is ring-capped exactly as a foreground
/// run's capture is: a runaway child cannot grow a run's memory without bound.
#[tokio::test]
async fn a_background_runs_unread_output_stays_inside_the_cap() {
    let Some(proven) = proven_shell("background-cap-output") else {
        return;
    };
    let id = proven
        .shell
        .start_background(background("yes . | head -c 20000000; echo done"))
        .await
        .expect("a background call starts");
    let report = proven
        .shell
        .read_background(id, Duration::from_secs(30))
        .await
        .expect("the run is known");
    assert_eq!(report.status, RunStatus::Exited { exit_code: Some(0) });
    assert!(
        report.truncated(),
        "twenty megabytes through a {OUTPUT_CAP_BYTES} byte ring must report truncation"
    );
    assert!(
        report.stdout.text.len() <= OUTPUT_CAP_BYTES
            && report.stderr.text.len() <= OUTPUT_CAP_BYTES,
        "the read stays inside the cap"
    );
}

/// A background call is never served by the foreground path: `run` refuses it
/// rather than quietly running it to completion.
#[tokio::test]
async fn a_background_call_is_never_served_by_a_foreground_run() {
    let Some(proven) = proven_shell("background-foreground") else {
        return;
    };
    let refused = proven.shell.run(background("true")).await;
    assert!(
        matches!(refused, Err(RunnerError::ShellCommandRefused { .. })),
        "a background call must not run through `run`: {refused:?}"
    );
}

/// A run this shell never started is unknown to it, and the refusal names the
/// id rather than reporting an empty read.
#[tokio::test]
async fn reading_a_run_this_shell_never_started_is_refused() {
    let Some(proven) = proven_shell("background-unknown") else {
        return;
    };
    let read = proven.shell.read_background(7, Duration::ZERO).await;
    assert!(
        matches!(read, Err(RunnerError::BackgroundUnknown { id }) if id == 7),
        "an unknown id is a typed refusal: {read:?}"
    );
    let killed = proven.shell.kill_background(7).await;
    assert!(matches!(killed, Err(RunnerError::BackgroundUnknown { id }) if id == 7));
}

/// A run that ends is noticed exactly once, without its output, and its
/// record stays readable so the notice's `shell_output` instruction is true.
#[tokio::test]
async fn an_exit_is_noticed_once_and_the_record_stays_readable() {
    let Some(proven) = proven_shell("background-notice") else {
        return;
    };
    let id = proven
        .shell
        .start_background(background("printf 'a\\nb\\n'"))
        .await
        .expect("a background call starts");
    let deadline = Instant::now() + Duration::from_secs(10);
    let exit = loop {
        if let Some(exit) = proven.shell.take_background_exits().into_iter().next() {
            break exit;
        }
        assert!(Instant::now() < deadline, "the run did not exit in time");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(exit.id, id);
    assert_eq!(exit.exit_code, Some(0));
    assert!(!exit.killed, "the command ended on its own");
    // Both streams count, and the sandbox lane may add a line of its own.
    assert!(
        exit.output_lines >= 2,
        "the command's two lines are counted: {}",
        exit.output_lines
    );
    assert!(
        proven.shell.take_background_exits().is_empty(),
        "an exit is reported once"
    );
    assert!(!proven.shell.background_live());
    assert!(!proven.shell.background_notice_pending());
    let read = proven
        .shell
        .read_background(id, Duration::ZERO)
        .await
        .expect("the noticed run is still readable");
    assert_eq!(read.status, RunStatus::Exited { exit_code: Some(0) });
    assert_eq!(read.stdout.text, "a\nb\n");
}
