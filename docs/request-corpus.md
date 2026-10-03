# Request corpus characterization

This is a deterministic, synthetic characterization of the current
`release/v0.4.2` request path at integration base `54f30d4d`. It is not a
provider benchmark and does not establish production token, cache, latency, or
correctness percentiles.

## Capture boundary

`actual_runtime_request_prefix_is_stable_and_last_sql_stays_in_the_user_tail`
in `crates/saya-cli/src/agent/runtime_budget_tests.rs` captures the exact
`ChatRequest` passed by `run_prompt_with_inputs` to a local `ChatProvider`.
It therefore includes the runtime-built system/context/history/user messages
and the actual ordered tool definitions and parameter schemas; it does not
reimplement request assembly. The provider returns a fixed local text response,
uses no network, and reports no usage.

The initial request in that fixture (one synthetic DuckDB profile, memory off,
no history, question plus last-SQL hint) measured:

| Component | Bytes |
| --- | ---: |
| Complete JSON-serialized `ChatRequest` | 14,227 |
| System message, role plus content | 2,239 |
| Both message role/content values | 2,782 |
| JSON-serialized tool list | 11,255 |

Measured with the temporary local capture command below before its diagnostic
was removed:

```sh
cargo test -p saya-cli --locked \
  actual_runtime_request_prefix_is_stable_and_last_sql_stays_in_the_user_tail -- --nocapture
```

These byte figures overlap by design: the complete JSON request includes JSON
syntax and both messages and tools, while the message figures use the
application's role/content accounting. They are not provider wire-byte counts.

## Corpus and existing proof

| Synthetic case | Evidence | Observed bounded outcome |
| --- | --- | --- |
| Straightforward query; repeated identical input | `actual_runtime_request_prefix_is_stable_and_last_sql_stays_in_the_user_tail` | Identical complete serialized requests. |
| Previous failed-SQL variation and changed question | Same test | System message and tool schemas unchanged; a failed-SQL hint and changed question remain in the user tail. |
| Large current tool schemas and non-ASCII question | Same test | Full tool set remains attached; `München 😀` reaches the user message. |
| Multi-step schema/query workflow | `turn_ceiling_stops_the_loop_at_the_planted_turn_count` | Three scripted query calls, then one tools-free salvage request. |
| Ambiguous meaning / clarification | `request_clarification_is_advertised_with_an_effect_none_declaration` and its execution tests | Clarification is an advertised no-effect tool and validates its arguments. |
| Malicious synthetic text | `injection_text_reaches_body_unmodified` | Stored text stays data in the bounded recall block; its wrapper/escaping is owned by message assembly. |
| Privacy, stale, confirmed/candidate selection | `privacy_off_produces_no_block_and_does_not_query_store`, `stale_claim_is_excluded_from_the_model_block`, and `confirmed_excludes_candidates_unchanged_behaviour` | Recall is privacy-gated, freshness-gated, and selects confirmed knowledge before candidates under the existing policy. |
| Pathological continuing calls | `turn_ceiling_stops_the_loop_at_the_planted_turn_count`, `tool_call_ceiling_stops_the_loop_at_the_planted_call_count`, `continuation_ceiling_bounds_the_truncation_retries` | The finite local scripts stop at 3 answering turns/3 calls/4 requests, 2 calls/4 requests, and 2 continuations/3 requests respectively. |

The last three tests use synthetic SQLite/demo data and a loopback mock HTTP
provider; they are not live providers or remote databases. They deliberately
cover finite scripted continuations, not an unbounded “endless” run. The
configured ceiling is the safety mechanism for a model that continues legal
calls forever.

## What was observable

For the static captured request: request count was 1, tool calls 0, retries 0,
and the local fixed response was useful by construction. No provider tokens,
cache reads, billed usage, first-useful-response latency, or provider
correctness/evidence fidelity were reported, so each is **unknown**, not zero.
No cache benefit is inferred from byte stability. Latest-request context
occupancy (the byte figures above) is distinct from accumulated billed usage,
which was unavailable.

The loopback ceiling corpus observed a maximum of 4 requests and 3 successful
tool calls in its finite cases. Its mock has no meaningful provider latency,
and the tests do not time it; duration is therefore unmeasured.

## Recommendation for the next budget decision

Use the measured finite workload as a minimum safety target: preserve explicit
limits no weaker than 3 answering turns, 3 tool calls, and 2 continuations for
an interactive default until a product decision replaces these characterization
values. Reserve 15% byte headroom over the 14,227-byte static baseline
(2,135 bytes; 16,362 bytes total) for message growth. Do not convert that
number to tokens or choose a wall-clock default from this corpus: token counts,
provider wire framing, caching, and duration were not observable. A future
provider-reported usage capture is required before setting a token-estimation
safety margin.
