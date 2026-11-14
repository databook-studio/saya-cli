# Coding diagnostic retention

Native root and child tool events, compaction observations, and run boundaries
are metadata-only diagnostics. They do not authorize approvals, effects, budget
changes or continuation. Mandatory journals retain their strict append policy.

Each diagnostic stream keeps its current JSONL file and one previous segment:

- `tool-events.jsonl` and `tool-events.previous.jsonl`
- `compactions.jsonl` and `compactions.previous.jsonl`
- `observations.jsonl` and `observations.previous.jsonl`

Each segment is at most 1 MiB and 65,536 records. A JSON object including its
newline framing is at most 8 KiB. Capacity rollover keeps whole records and
continues the run; evicted diagnostics do not consume execution authority.
The fixed retention summary and transaction staging file are each bounded to
16 KiB, and the stable lock file is empty. Maximum diagnostic disk space is
6 MiB + 32 KiB, including an interrupted transaction. No diagnostic record queue,
unbounded generation filenames or data-segment staging copies are used.

All writers, recovery and snapshots coordinate through `.diagnostics.lock`, which
is never renamed. A process mutex serializes local readers and writers for the
entire filesystem-lock lifetime. Cross-process lock acquisition is nonblocking;
external contention is a fixed typed fault. Job directories are private, owner-only directories; diagnostic files
are private regular files owned by the current user. Symlinks, hardlinks and
public permissions are refused. Cleanup only touches fixed diagnostic paths.

Before changing a segment, the writer syncs `diagnostics-summary.stage` and the
job directory. It then appends or rotates, syncs data and directory entries,
writes and syncs `diagnostics-summary.json`, and removes the stage marker with
another directory sync. An interrupted transaction or summary mismatch means
unknown loss and generation counts. Recovery removes only an incomplete final
record, preserving complete records; malformed complete records fail closed.
A valid legacy single-file journal remains readable and can rotate, but its
prior loss and generation identity are unknown. Resume preserves that uncertainty.

`saya job show`/status exposes `diagnostic_history`. `saya job export` includes
`artifacts.diagnostics`, containing the same summary and retained records ordered
previous segment then current segment within each stream. The active file alone
is no longer the complete history. Summary states have these meanings:

- `complete`: all diagnostics accounted for, with no known eviction.
- `truncated`: eviction occurred, with exact known evicted byte/record counts.
- `unknown`: legacy history, interrupted writes, or inconsistent summary prevent
  proving exact loss or generation order. Unknown counters are JSON null.

Diagnostic completeness is independent of job completion. Exported artifact
completeness is false when diagnostic history is truncated or unknown; durable
execution events and pagination retain their own completeness fields. Diagnostic
exports have a separate bounded allowance, so retained diagnostics do not exhaust
the existing mandatory-artifact export allowance.

Invalid/oversized records, unsafe paths, lock contention, storage and recovery
failures remain conservative typed failures. Secondary diagnostic failure cannot
replace a previously established execution cause or skip terminal settlement.
Only source-owned tool names, fixed outcomes/reasons, actual identities, the
bounded tool-result error text of a failed call and the bounded transcript of
each call are persisted. Provider reasoning, loop summaries and any content no
call produced remain excluded.

A failed, denied or refused tool event also records `at_unix_millis` (when the
outcome was observed), `arguments_sha256` (the lowercase-hex SHA-256 of the
serialized arguments the model sent — never the arguments), `failure_class`,
`error_text` and `reason_text`. `failure_class` and `error_text` are closed
vocabulary: the class is one of `approval_denied`, `admission_refused` or
`executor_error`, and the text is that class's fixed rendering. `reason_text` is
the tool-result error text the model itself received for that call — the
refusing rule's own wording, so a postmortem can name which rule refused a call
rather than only its class. It is the one field that can carry executor or
policy wording, paths or a provider message, so the store's own redaction runs
over it and it is cut to 1,024 bytes on a UTF-8 boundary with a trailing marker.
The closed `error_text` keeps its 200-byte bound. Records written before these
fields existed keep their old shape and still parse: the keys are simply absent,
and a succeeded or merely requested call carries none of them — a call that
succeeded records no `reason_text` at all.

`reason` is untouched: every row, old and new, still carries the closed outcome
word under that key — `dispatch_requested`, `executor_completed`,
`policy_denied`, `tool_failed` or `execution_refused`. The free wording lives
only under `reason_text`, so a reader that knows the word is never handed the
executor's text.

## Recorded tool calls

A tool row for a call the loop actually ran also carries `transcript`: the call's
own record, so a postmortem can tell which call touched which file instead of
only that a `write` happened.

- `call_id` — the id the provider gave the call, bounded to 128 bytes.
- `arguments` — the arguments the model sent, serialized.
- `result` — the result the tool returned, or the refusal or error text the model
  was handed back. Absent for a call refused before dispatch: nothing ran.
- `duration_ms` — how long the call took, from the loop's own view.

Every text is redacted exactly like the rest of the job's evidence — the store's
redaction runs before anything is written, so a credential inside a file body the
model wrote is scrubbed like one in a reason text. Each field is then bounded to
2 KiB, keeping both ends with an `...[elided]...` marker between them, and the
whole payload to 5 KiB; the writer re-bounds after redaction, which can lengthen
a text, and fits the record to the 8 KiB framing by cutting the fields again. A
transcript that still cannot fit is dropped whole — the row keeps its metadata
and its `reason_text`. A dispatch row (`status: requested`) never carries one: it
describes the same call before it ran.

One job records at most 256 KiB of transcript. When that ceiling is reached the
journal gains a single marker row — `tool: transcript`, `status: truncated`,
`reason: transcript_truncated`, `transcript_bytes` naming what was recorded —
and no later call records a transcript. The call metadata keeps being written
either way, so the journal stays a complete account of which calls ran. Rows
written before the transcript existed carry none of these keys and still parse;
readers that do not know them ignore the new object.
