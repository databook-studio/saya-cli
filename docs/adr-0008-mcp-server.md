# ADR 0008: The MCP stdio server — saya's tools for other hosts

- Status: accepted 2026-09-29. Records what shipped for release 0.4.2's MCP
  milestone (D), verified against the code; where the plan and the code
  differ, the code is recorded. Independent-client validation is recorded in
  §8 (opencode and codex-cli validated 2026-09-29; Claude Code not
  validated).
- Date: 2026-09-29
- Supersedes: nothing. Complements [ADR 0003](adr-0003-scratch-database.md)
  (the read-only gate every query crosses — MCP is one more client of it),
  [ADR 0004](adr-0004-saved-investigations-and-evidence.md) (the replay it
  exposes), and [ADR 0005](adr-0005-typed-query-parameters.md) (parameters —
  `investigation_run` binds them).
- Records why an MCP host can ask saya questions, why the profile allowlist
  and the data-sharing gate are decided at startup and never widened by a
  client, and why no approval or grant machinery exists on this surface.

## Context

MCP hosts — Claude Code, the MCP Inspector, any MCP client — want to reach
the capabilities saya already has: discover a schema, run a bounded
read-only query, read confirmed contracts, replay a saved investigation.
Exposing those over a wire protocol is a different risk shape from the TUI:
the peer is a program, not a person at a terminal. There is no one to
approve anything, no one to notice a widened permission, and every response
is data leaving the machine into another tool's context.

The design constraints therefore come from what already exists rather than
what MCP needs: every statement crosses the read-only AST gate
([ADR 0003](adr-0003-scratch-database.md)); confirmed claims are shown as
facts, never as authority ([ADR 0002](adr-0002-memory-and-contract-trust-model.md));
a saved replay never revalidates a stale review
([ADR 0004](adr-0004-saved-investigations-and-evidence.md)); and *rows* are
the one thing an MCP host cannot have unless the user deliberately allows
data to leave the machine — the same data-sharing choice that gates the AI
provider surface.

## Decision

### 1. What serves: rmcp over stdio, pinned and minimal

`saya mcp serve` serves the Model Context Protocol over stdio using
[`rmcp` 3.5.0](../Cargo.toml), pinned exactly, with only the `server` and
`transport-io` features — no macros feature; the handler is hand-written
(`rmcp-macros` and `schemars` arrive transitively through the `server`
feature, not enabled by choice). The server uses rmcp's default `serve()`
path: the legacy `initialize` handshake era, with no saya-side narrowing of
protocol versions — the negotiated version echoes whatever the client
requests that the server supports (pinned at `2025-06-18`).

stdout carries protocol frames and nothing else: dispatch happens before
any startup warnings are printed, the replay renders through the
process-output capture seam instead of stdout, and a single writer task owns
the stdout handle. Diagnostics — a startup banner naming the profile
allowlist and the data-sharing state, and runtime warnings — go to stderr.
The process exits 0 on stdin EOF; a refused startup (an unknown profile in
the allowlist) exits 2 with no protocol bytes written.

### 2. The profile allowlist is decided at startup and never widened

`--profile P` (repeatable, on the `serve` subcommand) fixes the allowlist;
with none given, the configured default profile is it. Every tool that takes
a profile is held inside it:
[`McpContext::allowed_profile`](../crates/saya-cli/src/mcp/context.rs) — a
membership check first, then the ordinary profile resolution — so a client
naming a configured-but-unlisted profile gets `profile not available: …` from
`schema`, `query`, `contracts`, and `investigation_run`'s `profile` argument,
and `list_profiles` never shows an unlisted profile.

`investigation_run` has no argument to hide behind: its effective target —
the `profile` argument, else the saved binding's profile, exactly the run
command's own resolution — is checked against the allowlist by
[`allowlist_gate`](../crates/saya-cli/src/mcp/replay_tools.rs) **before
anything runs**. A client-supplied name outside the allowlist is refused with
the same `profile not available: <name>` the other tools use; a binding
saved against a profile this server does not serve, or a binding that cannot
be read, refuses with a bare `profile not available` — no profile name
echoed, because the allowlist is also the client's information boundary
(`list_profiles` never names a profile outside it) and the gate fails closed.
With neither an argument nor a binding the gate passes and the run command
refuses with its own words, in its own order, before any connection. The
policy doc states the invariant plainly: decided once at startup, and no
entry in the server grants anything later.

### 3. Tools, and the data-sharing gate on rows

Five tools, all annotated read-only
([`catalog.rs`](../crates/saya-cli/src/mcp/catalog.rs)):

| Tool | Input | Returns | Gate |
| --- | --- | --- | --- |
| `list_profiles` | none (arguments refused) | allowlist names + dialects only | always |
| `schema` | `profile` | live discovery with the state-store cache fallback | always |
| `contracts` | `profile`, `table?` (`catalog.schema.object`, exactly three parts) | **Active claims only**, through the identity-dropping contract view; absent contract → an empty `claims` list with a note | always |
| `query` | `profile`, `sql` | bounded rows (`max_rows` from config), `columns`/`rows`/`truncated`, evidence with no connection identity | **data sharing on** |
| `investigation_run` | `id`, `profile?`, `params?` (`{"name": "value"}` strings) | the replay's `{result, evidence, connection}` | **data sharing on** |

The data-sharing gate is the config's `[ai] allow_data_sharing` (default
**false**), folded with the global `--allow-data-sharing` flag (`--no-data-sharing`
wins); it is not a serve-local flag. Row-returning tools are **absent
entirely** from `tools/list` when sharing is off — not merely marked — and
the dispatch refuses them too, so a stale tool list cannot be exploited.

`investigation_run` reuses the CLI's typed replay operation with
`revalidate` hardcoded `false`: a stale review is an `isError` result
carrying the CLI's own message (`review is stale (…); pass --revalidate …`
— words the MCP client reads, a flag it cannot pass), and a successful MCP
replay never rewrites the review binding. Declared parameters bind through
an optional `params` map of `{"name": "value"}` strings — the same
`name=value` grammar the CLI's `--param` parses (the literal `null` binds a
typed null, values are parsed as their declared types, and a missing
required parameter is refused before any connection) — read by the same
string-map helper the other tools share, so [ADR 0005](adr-0005-typed-query-parameters.md)'s
value-never-stored property holds here too.

### 4. No approvals, no grants, no prompts

The connector factory is built with `can_prompt = false`: when a secret is
missing, MCP refuses — it never prompts, the same posture headless runs
take. There is no approval machinery on this surface at all — the agent
loop's approval modes are not read here — and no grant or write capability:
every tool is a read-shaped query against the existing gates. A client
cannot escalate by asking; the surfaces that can be granted (approvals,
grants, plan scopes) are not reachable from the protocol.

### 5. Bounds

- **Request ≤ 1 MiB**, enforced by a bounded stdin **line gate** in the
  transport ([`line_gate.rs`](../crates/saya-cli/src/mcp/line_gate.rs)): an
  oversized line is discarded unread and answered once, at its newline, with
  JSON-RPC error `-32600` (`request too large`) and `id: null` — the id is
  unknown because the line was never read. The server keeps serving. (A
  defense-in-depth re-check inside the tool handler returns `-32602` with
  the real id if arguments somehow exceed the cap.) rmcp's own stdio
  transport is replaced by a bridge, because it has no inbound cap.
- **≤ 4 tool calls in flight**: over the cap the call gets a tool-level
  `isError` result — `busy: too many in-flight tool calls; retry once one
  completes` — not a protocol error, so a client's request is never
  silently lost.
- **30 s per call**: expiry yields a tool-level `isError` naming the bound.
- **Response ≤ 16 MiB**: an over-bound payload is an `isError`; `query`
  first narrows by halving rows (with a note saying the response was cut to
  N/M rows), refusing outright only when even the rowless payload exceeds
  the bound.
- Read chunks are 8 KiB and the frame channel depth is 64, so a chatty
  client bounds memory.

### 6. Cancellation drops the call

A `notifications/cancelled` stops the wait and drops the call's future; the
late response is discarded by the service, and the client gets no answer for
the cancelled request (pinned). Server-side cancel happens only where the
connector supports it and only on the `query` path: the connector race
attempts `cancel()` — PostgreSQL (`pg_cancel_backend`), MySQL (`KILL` on a
short-lived connection), Snowflake (the statement-handle endpoint), SQLite
(an execute-loop flag), and DuckDB (an interrupt handle). ClickHouse and
BigQuery have no cancellation and this is stated rather than hidden. A
cancelled `investigation_run` releases its output-capture slot and relies on
future drop; it does not cancel the server-side query.

### 7. Errors are sanitized `isError` results

Tool failures return `isError` tool results whose text passes through
redaction then terminal sanitization — credential-shaped content
(`password=`, `token=`, `secret=`, authorization headers, PEM private-key
blocks, URL userinfo) becomes `[redacted]`, and control characters are
stripped. No filesystem path appears in a rendered failure (pinned). The
protocol-level errors (`-32600`, `-32601` unknown tool, `-32602`) are static
text by construction.

### 8. Validation

The integration suite
([`tests/mcp_stdio.rs`](../crates/saya-cli/tests/mcp_stdio.rs)) drives the
**real binary** over piped stdio, speaking newline-delimited JSON-RPC —
stdout-purity across a full happy path, allowlist behaviour and the
cannot-widen pin, the `-32600` oversize gate, the shared safety path (a
`DELETE` refused with the gate's words), the data-sharing matrix (off /
config-on / `--no-data-sharing`), cancellation and concurrency bounds, and
the replay happy path and stale-review refusal. Unit tests pin the policy
constants, the allowlist resolution, the catalog's data-sharing gating, and
the line gate's state machine. The plan's "in-process rmcp client" test did
not ship — the binary-driven suite is what exists.

**Validated against real clients, 2026-09-29.** The maintainer drove the
server with two independent MCP clients:

- **opencode 1.18.31** — configured via the `opencode.json` `mcp` block
  (`type: "local"`). list / schema / query all worked; a `DELETE` was
  refused by the read-only gate; with data sharing off the `query` tool was
  hidden from `tools/list`.
- **codex-cli 0.155.1** — configured via `mcp_servers.saya.*` (config.toml
  or `-c` overrides). Same results: list / schema / query ok, `DELETE` and
  `DROP` refused, data-sharing-off hides query.
- **Claude Code was not validated** — the validation session was not signed
  in, so no run was performed with it. The `claude mcp add saya -- saya mcp
  serve …` setup shape is documented in [commands](commands.md) but
  unverified by a real client session.

## Consequences

**Accepted costs.**

- The allowlist is startup-fixed: adding a profile means restarting the
  server. Deliberate — a per-request widening is exactly the escalation a
  wire peer must never be able to do.
- `can_prompt = false` means an auth path that would interactively prompt
  (Snowflake's browser flow, a missing env var) refuses instead. MCP users
  need complete, resolvable credentials.
- The hand-written handler (no macros) is more code than `#[tool]` derives
  would be, in exchange for a dependency surface of two features and a
  schema writer the server controls.
- Rows are invisible by default. An MCP host that wants query results needs
  the operator to start the server with `--allow-data-sharing` or set
  `[ai] allow_data_sharing` — an explicit act by the person who owns the
  data, which is the point.
- Cancellation is wait-abandonment by default; only `query` on five engines
  stops the actual server-side work. A cancelled long query on BigQuery or
  ClickHouse keeps running there.

**Rejected alternatives.**

- *rmcp's own stdio transport.* It has no inbound cap; a client could push
  an unbounded line into memory before any handler saw it. The bounded line
  gate exists because the cap must hold before parse, not after.
- *Sharing the agent loop's approval machinery.* Approval needs a human at
  a terminal; MCP has none by construction. Wiring approval prompts into the
  protocol would either block forever or widen the trust surface to the
  host — both worse than refusing to have the machinery.
- *Serving rows tools "marked unavailable" when sharing is off.* A client
  that lists the tool and gets a refusal per call is being invited to retry;
  absence from `tools/list` plus a dispatch-side refusal states the policy
  once and cannot drift between the two checks.
- *Letting `investigation_run` pass `--revalidate`.* A stale review is the
  product telling the user the world changed; a wire client that could
  clear it remotely would reduce review to a formality. The refusal text
  crosses the wire; the flag does not.

## Limitations (stated, not solved)

- stdio only — no HTTP/SSE transport; one host per server process.
- Requests still pending when stdin reaches EOF are **dropped, not
  answered**: the server exits 0 on EOF without waiting for in-flight work,
  so a one-shot pipeline that writes its JSON-RPC frames and closes stdin
  gets no reply to the frames still in flight — a host keeps stdin open for
  the session's life and gets every reply (the concrete one-shot/hold-open
  shapes are in [commands](commands.md)).
- Claude Code is documented (`claude mcp add saya -- saya mcp serve …`) but
  was not validated by a real client session; see §8.
- `contracts` returns Active claims only — reviewing a Pending claim still
  happens in the TUI.
- The 30 s call bound is the ceiling for every tool; long-running queries
  need the config's `query_timeout_seconds` lowered, not the MCP bound
  raised.
- Responses to cancelled requests are dropped; the client sees the
  cancellation, not a result.
- Protocol versions are rmcp's defaults, not narrowed by saya; the newest
  rmcp-known era is advertised even for handshake clients.