//! Two `helpers` calls of one run must be able to run at the same time, each
//! `task_run` returning its OWN helper's summary, and a helper over the child
//! bound must WAIT for a slot instead of being refused.
//!
//! The helpers run against a scripted provider: it gates each child's first
//! turn so the overlap is observable and, with a bounded hold, so the refused
//! case FAILS on a timeout instead of hanging.
#![cfg(unix)]

use super::CodingError;
use super::direct_root_fixture::{
    RootFixture, fixture_with_endpoint, fixture_with_endpoint_and_agents,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

/// The whole `tokio::join!` of two `task_run` calls must settle well inside
/// this: the overlap gate releases after its bounded hold, so a refusal is a
/// failure, never a hang.
const RUN: Duration = Duration::from_secs(30);

/// How long the overlap gate holds a child's turn waiting for its sibling. The
/// gate releases as soon as two child turns arrive, so this only bounds the wait
/// when the second never arrives: it must outlast two staged-and-admitted
/// `helpers` calls, which is seconds, not milliseconds.
const HOLD: Duration = Duration::from_millis(10_000);

/// How long each child's turn is held in the bound scenario, so two helpers
/// *would* overlap if the child bound did not serialise them.
const DELAY: Duration = Duration::from_millis(200);

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Hold every child turn until two have arrived, then release both.
    Overlap,
    /// Hold every child turn briefly; record the peak concurrent children.
    Serial,
    /// Fail `helper-2` outright; answer the rest.
    OneFails,
}

/// A local scripted provider: every request is a child turn. It answers with
/// `"<agent> finished its piece"` so a returned summary names its own helper.
struct ScriptedProvider {
    endpoint: String,
    overlapped: Arc<AtomicBool>,
    peak: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ScriptedProvider {
    fn start(mode: Mode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let overlapped = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (seen, high, done) = (
            Arc::clone(&overlapped),
            Arc::clone(&peak),
            Arc::clone(&stop),
        );
        let worker = thread::spawn(move || {
            let mut pending = Vec::<(TcpStream, String, Instant)>::new();
            let mut high_water = 0usize;
            while !done.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        if let Some(body) = read_request(&mut stream) {
                            let name = helper_name(&body).unwrap_or_else(|| "helper-?".into());
                            let response = if mode == Mode::OneFails && name == "helper-2" {
                                http(500, "")
                            } else {
                                http(200, &answer(&format!("{name} finished its piece")))
                            };
                            let hold = match mode {
                                Mode::Overlap => HOLD,
                                Mode::Serial => DELAY,
                                Mode::OneFails => Duration::ZERO,
                            };
                            if hold.is_zero() {
                                send_raw(&mut stream, &response);
                            } else {
                                pending.push((stream, response, Instant::now() + hold));
                            }
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(_) => break,
                }
                if mode == Mode::Overlap && pending.len() >= 2 {
                    seen.store(true, Ordering::Release);
                    for (mut stream, response, _) in pending.drain(..) {
                        send_raw(&mut stream, &response);
                    }
                }
                let now = Instant::now();
                let mut index = 0;
                while index < pending.len() {
                    if now >= pending[index].2 {
                        let (mut stream, response, _) = pending.remove(index);
                        send_raw(&mut stream, &response);
                    } else {
                        index += 1;
                    }
                }
                high_water = high_water.max(pending.len());
                high.store(high_water, Ordering::Release);
                thread::sleep(Duration::from_millis(2));
            }
        });
        Self {
            endpoint,
            overlapped,
            peak,
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for ScriptedProvider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

fn read_request(stream: &mut TcpStream) -> Option<String> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let count = stream.read(&mut buffer).ok()?;
        if count == 0 {
            return None;
        }
        bytes.extend_from_slice(&buffer[..count]);
        assert!(bytes.len() <= 1024 * 1024);
        if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let header = String::from_utf8_lossy(&bytes[..index]);
            let length = header.lines().find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })?;
            if bytes.len() >= index + 4 + length {
                return String::from_utf8(bytes[index + 4..index + 4 + length].to_vec()).ok();
            }
        }
    }
}

/// The helper name a child's turn belongs to: the assignment text carries
/// `write for <agent>`.
fn helper_name(body: &str) -> Option<String> {
    let rest = &body[body.find("helper-")?..];
    let end = rest
        .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-'))
        .unwrap_or(rest.len());
    Some(rest[..end].to_owned())
}

fn http(status: u16, body: &str) -> String {
    format!(
        "HTTP/1.1 {status} Status\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn send_raw(stream: &mut TcpStream, raw: &str) {
    let _ = stream.write_all(raw.as_bytes());
    let _ = stream.flush();
}

fn answer(text: &str) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":"stop"}]}),
        json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5}})
    )
}

fn helper_args(agent: &str) -> Value {
    helper_args_of("general", agent)
}

/// One `helpers` call of the named helper type.
fn helper_args_of(subagent_type: &str, agent: &str) -> Value {
    json!({
        "description": format!("write for {agent}"),
        "prompt": format!("write for {agent}"),
        "subagent_type": subagent_type,
        "agent_id": agent,
    })
}

/// The one summary a `task_run` returns (its own helper's).
fn summary_json(result: &Value) -> &Value {
    result
        .as_array()
        .expect("a helpers call returns an array of summaries")
        .first()
        .expect("a helpers call returns exactly one summary")
}

fn summary_of(result: &Value) -> String {
    summary_json(result)["summary"]
        .as_str()
        .expect("the summary carries a string")
        .to_owned()
}

async fn job(label: &str, provider: &ScriptedProvider, max_children: Option<usize>) -> RootFixture {
    fixture_with_endpoint(
        label,
        vec!["tracked.txt".into()],
        &provider.endpoint,
        max_children,
    )
    .await
}

async fn join_two(f: &RootFixture) -> (Value, Value) {
    let (first, second) = tokio::time::timeout(RUN, async {
        tokio::join!(
            f.job.task_run(helper_args("helper-1")),
            f.job.task_run(helper_args("helper-2")),
        )
    })
    .await
    .expect("both helpers calls must settle, not hang");
    (
        first.expect("helper-1's call must not be refused with Busy"),
        second.expect("helper-2's call must not be refused with Busy"),
    )
}

#[tokio::test]
async fn two_helpers_calls_run_at_the_same_time_and_each_returns_its_own_summary() {
    let provider = ScriptedProvider::start(Mode::Overlap);
    let f = job("concurrency-overlap", &provider, None).await;
    let (first, second) = join_two(&f).await;
    assert!(
        provider.overlapped.load(Ordering::Acquire),
        "both helpers must be in flight together, not run serially"
    );
    assert_eq!(summary_of(&first), "helper-1 finished its piece");
    assert_eq!(summary_of(&second), "helper-2 finished its piece");
}

#[tokio::test]
async fn a_helper_over_the_child_bound_waits_for_a_slot_and_never_overlaps() {
    let provider = ScriptedProvider::start(Mode::Serial);
    let f = job("concurrency-bound", &provider, Some(1)).await;
    let (first, second) = join_two(&f).await;
    assert_eq!(
        provider.peak.load(Ordering::Acquire),
        1,
        "a child bound of 1 must keep the two helpers from overlapping"
    );
    assert_eq!(summary_of(&first), "helper-1 finished its piece");
    assert_eq!(summary_of(&second), "helper-2 finished its piece");
}

#[tokio::test]
async fn one_failing_helper_does_not_stop_its_sibling_from_returning_its_own_summary() {
    let provider = ScriptedProvider::start(Mode::OneFails);
    let f = job("concurrency-failure", &provider, None).await;
    let (first, second) = join_two(&f).await;
    assert_eq!(summary_of(&first), "helper-1 finished its piece");
    assert_eq!(
        summary_json(&second)["outcome"],
        "failed",
        "the failing helper reports its own failure, and its sibling still completed"
    );
}

/// Two `helpers` calls of one turn naming the SAME agent: exactly one is
/// admitted, the other is refused `InvalidRequest`, and `tasks.jsonl` holds one
/// record for that agent. Admission is a check-then-act over the task history —
/// `validate_delegate_tasks` reads `tasks.jsonl` and `stage_delegation` appends
/// to it — so the two calls must not interleave between the read and the append.
#[tokio::test]
async fn two_helpers_calls_naming_the_same_agent_admit_exactly_one() {
    let provider = ScriptedProvider::start(Mode::Serial);
    let f = job("concurrency-duplicate-agent", &provider, None).await;
    let (first, second) = tokio::time::timeout(RUN, async {
        tokio::join!(
            f.job.task_run(helper_args("helper-1")),
            f.job.task_run(helper_args("helper-1")),
        )
    })
    .await
    .expect("both helpers calls must settle, not hang");

    let outcomes = [first, second];
    let admitted = outcomes.iter().filter(|outcome| outcome.is_ok()).count();
    assert_eq!(
        admitted, 1,
        "exactly one of the two calls naming one agent must be admitted: {outcomes:?}"
    );
    let refused = outcomes
        .iter()
        .find(|outcome| outcome.is_err())
        .expect("one call was refused");
    assert!(
        matches!(refused, Err(CodingError::InvalidRequest)),
        "the refused call must report the assignment as invalid: {refused:?}"
    );

    let history = fs::read_to_string(f.job.records.join("tasks.jsonl")).unwrap();
    let records: Vec<Value> = history
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        records.len(),
        1,
        "one admitted helper writes one task-history record: {records:?}"
    );
    assert_eq!(records[0][0]["agent_id"], "helper-1", "{records:?}");
}

/// A configured helper type is admitted end to end — the registry is what the
/// job advertises — and the `[agents] max_children` bound stated in the same
/// table still caps how many helpers run at once: a new type is configuration,
/// not a new concurrency rule, and the bound the table states is the bound the
/// scheduler runs the job under.
#[tokio::test]
async fn a_configured_type_runs_and_the_child_bound_still_caps_concurrency() {
    let provider = ScriptedProvider::start(Mode::Serial);
    let f = fixture_with_endpoint_and_agents(
        "concurrency-configured",
        vec!["tracked.txt".into()],
        &provider.endpoint,
        "[agents]\nmax_children = 1\n\n[agents.scout]\nrole = 'analyst'\n",
    )
    .await;
    let (first, second) = tokio::time::timeout(RUN, async {
        tokio::join!(
            f.job.task_run(helper_args_of("scout", "helper-1")),
            f.job.task_run(helper_args_of("scout", "helper-2")),
        )
    })
    .await
    .expect("both helpers calls must settle, not hang");
    assert_eq!(
        provider.peak.load(Ordering::Acquire),
        1,
        "a child bound of 1 must keep two configured helpers from overlapping"
    );
    assert_eq!(
        summary_of(&first.expect("helper-1 must run")),
        "helper-1 finished its piece"
    );
    assert_eq!(
        summary_of(&second.expect("helper-2 must run")),
        "helper-2 finished its piece"
    );
}
