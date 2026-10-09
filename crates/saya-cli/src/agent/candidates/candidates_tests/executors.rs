/// A scripted `CandidateExecutor`: exact-SQL lookup returns `Some(result)`;
/// anything else returns `None` (a failed statement). Mirrors `ScriptedExecutor`
/// in `decide_tests.rs`.
struct ScriptedExecutor {
    results: HashMap<String, QueryResult>,
}

#[async_trait]
impl CandidateExecutor for ScriptedExecutor {
    async fn run(&self, sql: &str) -> Option<QueryResult> {
        self.results.get(sql).cloned()
    }
}

/// A `CandidateExecutor` that only counts calls and returns nothing. The vote
/// segment is the only orchestrate path that calls the executor, so a zero
/// count is the proof that no vote segment ever began.
struct CountingExecutor {
    calls: AtomicUsize,
}

impl CountingExecutor {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl CandidateExecutor for CountingExecutor {
    async fn run(&self, _: &str) -> Option<QueryResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        None
    }
}

/// A scripted `CandidateExecutor` that serves its result and latches the
/// question token with a stop reason on every call — the verification work
/// itself is where the stop fired, the way a deadline expiring mid-decision
/// would.
struct StopDuringVoteExecutor {
    calls: AtomicUsize,
    results: HashMap<String, QueryResult>,
    reason: CancellationReason,
    token: CancellationToken,
}

impl StopDuringVoteExecutor {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl CandidateExecutor for StopDuringVoteExecutor {
    async fn run(&self, sql: &str) -> Option<QueryResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.token.cancel_with_reason(self.reason);
        self.results.get(sql).cloned()
    }
}

// ---------------------------------------------------------------------------
// recording sink
// ---------------------------------------------------------------------------

struct RecordingSink {
    events: Arc<Mutex<Vec<AgentEvent>>>,
}

#[async_trait]
impl AgentEventSink for RecordingSink {
    async fn emit(&self, event: AgentEvent) {
        self.events.lock().expect("events lock").push(event);
    }
}
