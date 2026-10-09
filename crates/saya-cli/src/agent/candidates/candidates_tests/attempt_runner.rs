/// One attempt's scripted outcome: either an `AgentOutput` (carrying nominated
/// SQL) or an error. Stored in a queue and moved out one per `run()` call so
/// the non-`Clone` error type needs no cloning.
enum Outcome {
    Ok(AgentOutput),
    Err(AgentRuntimeError),
}

/// A scripted `AttemptRunner`: returns one queued outcome per call, records
/// every call, and optionally latches the token with a stop reason once
/// `cancel_after` calls have completed so a stop-between-attempts test can
/// fire the token exactly where a real budget or signal would.
struct ScriptedRunner {
    outcomes: Mutex<Vec<Outcome>>,
    calls: AtomicUsize,
    cancel_after: Option<(usize, CancellationToken, CancellationReason)>,
}

impl ScriptedRunner {
    fn new(outcomes: Vec<Outcome>) -> Self {
        Self {
            outcomes: Mutex::new(outcomes),
            calls: AtomicUsize::new(0),
            cancel_after: None,
        }
    }

    fn cancel_after(self, n: usize, token: CancellationToken) -> Self {
        self.cancel_after_with(n, token, CancellationReason::User)
    }

    fn cancel_after_with(
        mut self,
        n: usize,
        token: CancellationToken,
        reason: CancellationReason,
    ) -> Self {
        self.cancel_after = Some((n, token, reason));
        self
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl AttemptRunner for ScriptedRunner {
    fn run(&self) -> Pin<Box<dyn Future<Output = Result<AgentOutput, AgentRuntimeError>> + '_>> {
        Box::pin(async move {
            let count = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if let Some((n, token, reason)) = &self.cancel_after
                && count == *n
            {
                token.cancel_with_reason(*reason);
            }
            let mut outcomes = self.outcomes.lock().expect("outcomes lock");
            match outcomes.len() {
                0 => panic!("ScriptedRunner ran out of outcomes (call #{count})"),
                _ => match outcomes.remove(0) {
                    Outcome::Ok(out) => Ok(out),
                    Outcome::Err(e) => Err(e),
                },
            }
        })
    }
}
