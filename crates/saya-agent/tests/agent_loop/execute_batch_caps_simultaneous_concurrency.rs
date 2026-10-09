/// Proves the fan-out in `execute_batch` is bounded: every executor entry
/// increments an in-flight counter, parks on a zero-permit semaphore so peers
/// can pile up, and records the high-water mark of simultaneous execution. The
/// test waits for in-flight to stabilise (the cap's worth are parked), reads
/// the high-water, and releases. Unbounded simultaneity drives the high-water
/// to N; a cap of C holds it at C.
#[tokio::test]
async fn execute_batch_caps_simultaneous_concurrency() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::sync::Semaphore;

    struct ProbeTools {
        in_flight: Arc<AtomicUsize>,
        high_water: Arc<AtomicUsize>,
        gate: Arc<Semaphore>,
    }

    #[async_trait::async_trait]
    impl ToolExecutor for ProbeTools {
        async fn execute(
            &self,
            _: &str,
            _: serde_json::Value,
        ) -> Result<serde_json::Value, ToolError> {
            let cur = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            let mut seen = self.high_water.load(Ordering::SeqCst);
            while cur > seen {
                match self.high_water.compare_exchange(
                    seen,
                    cur,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => break,
                    Err(actual) => seen = actual,
                }
            }
            // Block on a zero-permit gate, holding the in-flight slot open so
            // the next call can only start once a slot frees. The test adds the
            // permits that release every parked call.
            let _permit = self.gate.acquire().await.expect("gate not closed");
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            Ok(serde_json::json!({"ok": true}))
        }
    }

    const N: usize = 12;
    let provider = MockProvider {
        responses: Mutex::new(vec![
            ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: (0..N)
                    .map(|index| ToolCall {
                        id: format!("call-{index}"),
                        name: "schema_discovery".into(),
                        arguments: serde_json::json!({"which": index}),
                    })
                    .collect(),
                tool_call_id: None,
            }),
            ChatResponse::new(ChatMessage::text("assistant", "done")),
        ]),
    };
    let in_flight = Arc::new(AtomicUsize::new(0));
    let high_water = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new(Semaphore::new(0));
    let tools = ProbeTools {
        in_flight: in_flight.clone(),
        high_water: high_water.clone(),
        gate: gate.clone(),
    };
    let run = run_agent(
        &provider,
        &tools,
        request(),
        definitions(),
        AgentLimits {
            max_turns: Some(4),
            max_tool_calls: Some(64),
            max_continuations: None,
            permit_candidate_writes: false,
            context_byte_budget: 1024 * 1024,
            permit_workspace_writes: false,
            permit_external_effects: false,
        },
        &AllowReadOnlyApproval,
    );
    // Drive the run while waiting for in-flight to stabilise — i.e. the cap's
    // worth are parked and the rest are queued behind the cap. Polling both
    // futures on this task avoids borrowing across a `tokio::spawn` boundary.
    let mut run = std::pin::pin!(run);
    let stabilised = tokio::select! {
        output = &mut run => { let _ = output; false },
        _ = async {
            let mut last = 0;
            let mut stable = 0;
            loop {
                let cur = in_flight.load(Ordering::SeqCst);
                if cur == last && cur > 0 {
                    stable += 1;
                    if stable >= 3 {
                        return;
                    }
                } else {
                    stable = 0;
                }
                last = cur;
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        } => true,
    };
    assert!(
        stabilised,
        "in-flight must stabilise under the cap, not finish"
    );
    let observed = high_water.load(Ordering::SeqCst);
    // Release every parked call so the run can drain and finish. Permits
    // accumulate, so one add covers all N calls regardless of how the cap
    // batches them.
    gate.add_permits(N);
    let output = tokio::time::timeout(Duration::from_secs(10), run)
        .await
        .expect("batch must not deadlock under the cap")
        .unwrap();
    assert_eq!(output.answer, "done");
    // Before the fix the high-water mark equalled N (all twelve ran at once);
    // the cap must hold the observed simultaneity strictly below N.
    assert!(
        observed < N,
        "concurrency must be capped below {N}; observed {observed} simultaneous"
    );
    assert!(
        observed >= 1,
        "at least one call must run; observed {observed}"
    );
}
