# C3 candidate receipt report

## SPEC REVIEW

`run_with_candidates` now wraps the supplied sink once for the candidate
question. The wrapper records only actual answering `Usage` and
`FailedAttemptUsage` events, forwards every event and `observe_answering`
unchanged, and the selected candidate receives that aggregate once. It does
not fold `output.usage` again. The shared arithmetic is also now the
`TerminalSink` arithmetic: scalar fields saturate; optional details remain
`None` until reported, while a reported zero remains `Some(0)`.

The obsolete cfg(test) orchestrator and optional-scope inner path are removed.
There is one production `orchestrate` path requiring `QuestionScope`; its
existing fake-runner voting tests construct that real scope through a local
test helper. No candidate voting, token limits, elapsed-time, provider,
runtime, TUI, or session lifecycle policy changed.

## Producer regression

`agent::candidates::receipt_tests::failed_candidate_receipt_survives_later_sqlite_answer_once`
uses `run_with_candidates` → `LiveAttemptRunner` → the production runtime,
with a local HTTP OpenAI-compatible source and a seeded SQLite `orders` table.
The source serves three actual requests: a duplicate-tool-ID response rejected
by protocol validation after usage `11/13`, then a valid SQLite tool response
with `17/19`, then a natural answer with `23/29`. It proves three requests, a
real `bounded_sql_query` completion, selected text `The query returned one.`,
and exact output receipts of **51 input** and **61 output** tokens.

Red, before production code:

```text
CXXFLAGS=-g0 cargo test -p saya-cli --lib failed_candidate_receipt_survives_later_sqlite_answer_once --locked
1 failed / 2084 filtered: input 40, expected 51
```

Green after the fix:

```text
CXXFLAGS=-g0 cargo test -p saya-cli --lib failed_candidate_receipt_survives_later_sqlite_answer_once --locked
1 passed / 2084 filtered

CXXFLAGS=-g0 cargo test -p saya-cli --lib agent::candidates --locked
10 passed / 2075 filtered
```

Test audit: the new test observes the production provider/runtime/sink path,
asserts externally meaningful receipts and request/tool outcomes, uses bounded
accept/read/run timeouts, and does not hand-emit the receipt under test or use
a fake attempt runner for the producer claim. The migrated fake-runner tests
continue to own only voting mechanics; their receipt test emits normal usage
events into the real wrapper rather than trusting fake output totals.

## Gates and limits

Passed on the final code: `cargo fmt --check`; strict CLI all-target clippy;
workspace all-target check; and `git diff --check` (all with
`CXXFLAGS=-g0` where applicable).

The full `CXXFLAGS=-g0 cargo test -p saya-cli --locked` run reached the 2,085
library-test pass and then failed one unrelated existing MVP SIGINT session
test: `session::active_sigint_cancels_request_without_persisting_incomplete_turn`.
It failed twice without source edits: first with the local provider read
returning `WouldBlock`, then with its expected stream-interruption transcript
mismatch. This packet does not change session/provider lifecycle behavior.

Code commit: `508dce29 fix(cli): retain failed candidate receipts`.

Candidate source/time proofs and TUI/headless lifecycle work remain next. This
does not claim completion of all C3 or all14 work.
