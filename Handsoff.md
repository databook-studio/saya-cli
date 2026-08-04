# SAYA CLI — Upliftment Hand-off Log

> Autonomous execution log. Implementation engine: **`agy` (Antigravity CLI, `gemini-3.6-flash-high`)**.
> Orchestration & verification: Claude Code. Reference for domain patterns: DataBook Studio
> (`/Users/subodhsharma/Projects/DataTalkie/src-tauri/src/features/saya_ai`).
>
> Standards enforced every ticket (databook `CODING_STANDARDS.md` + `CLAUDE.md`):
> thiserror errors · async-trait trait methods · no `unwrap`/`expect` in lib code ·
> `Result<T,E>` propagation · rustdoc on public fns · in-file `#[cfg(test)]` + `tests/` ·
> no stray `println!` · files ≤150 target / 250 hard ceiling · conventional commits.
>
> Verify gate per ticket: `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings` + `cargo test --workspace` all green.

## Status legend
`TODO` · `IN-PROGRESS` · `VERIFY` (agy done, verifying) · `DONE` · `BLOCKED`

---

## Phase A — Multi-database navigation
- [x] **A1** `DONE` — saya-cli `connection_registry.rs`: typed `ConnectionRegistry` (primary + name→entry map, `resolve`, `describe_context`). Verified: fmt + clippy + full workspace test green.
- [x] **A2** `DONE` — `agent_tools.rs`: `DatabaseTools` holds registry; `schema_discovery`/`bounded_sql_query` gain optional `connection` param; executor routes via `registry.resolve`. Normalized module to top-level `crate::connection_registry` (agy had nested it under agent_tools). Verified: fmt + clippy --all-targets + full test green.

  NOTE: agy reverted my mvp.rs flake fix during A3 (out-of-scope edit). Reapplied + re-verified. Now forbidding agy from editing out-of-scope files in every prompt, and re-checking the flake fix after each agy run.
- [x] **A3** `DONE` — saya-agent: `AgentRequest.system_prompt` appended to existing base SYSTEM_PROMPT; `build_messages` gains `system_extra` param. All `AgentRequest` literals updated. Verified green.
- [x] **A4** `DONE` — `agent_runtime.rs` + new `connection_build.rs` (decoupled, unit-tested with DuckDB temp files): build primary + included connectors → registry; inject `describe_context()` as `system_prompt`; `profile_names = registry.names()`; secondary build/connect failure = soft-skip (fail-fast, no interactive auth for secondaries); `PromptOverrides.included_profiles`; `prompt_overrides()` fills it. Removed now-dead `with_state` shim + gated test-only imports. Verified: fmt + clippy --all-targets + 160 tests green.
- [x] **A5a** `DONE` — threaded `--include-profile` into non-interactive `ask` (app → commands::run → ask → PromptOverrides.included_profiles). Verified green.
- [x] **A5b** `DONE` — integration test `ask_navigates_between_included_database_connections` (mvp.rs): scripted openai_compatible server issues two `schema_discovery` calls (primary default + `{"connection":"warehouse"}`); asserts 2 distinct profile-id audits. Authored by Claude as the verification step (protects the fragile mvp.rs; all feature code A1–A5a written by agy). Passes in isolation + full suite (161 tests).
- [x] **A6** `DONE` — docs: querying-databases.md (+ "Querying multiple databases" section), commands.md (`--include-profile`, `/include`, `/exclude`). Reviewed for accuracy (agent-navigation vs. not-federation clearly stated).

### ✅ PHASE A COMPLETE — multi-database navigation shipped & proven end-to-end (161 tests green).

## Phase B — Anthropic provider
- [x] **B1** `DONE` — providers/anthropic{,_request,_stream}.rs + tests/anthropic.rs (SSE content_block parsing, input_schema tools, top-level system, x-api-key + anthropic-version, empty tool input → {}). Verified: fmt + clippy --all-targets + 164 tests green; files ≤250.
- [x] **B2/B3** `DONE` — agent_provider::build Anthropic branch + query_data_allowed (cloud, sharing-gated) + privacy test `anthropic_is_cloud_gated_on_data_sharing` + providers.md. Verified: fmt + clippy --all-targets + 165 tests green.

### ✅ PHASE B COMPLETE — Anthropic provider shipped (native Claude support).

## Phase C — Gemini provider
- [x] **C1** `DONE` — providers/gemini{,_request,_response}.rs + tests/gemini.rs (buffered complete(), x-goog-api-key, systemInstruction, functionDeclarations, id→name resolution for functionResponse). Verified: fmt + clippy --all-targets + 171 tests green; files ≤250.
- [x] **C2/C3** `DONE` — agent_provider::build Gemini branch (removed unreachable `other` fallback) + query_data_allowed Gemini (removed unreachable `_`) + privacy test + docs. Footgun resolved. Verified: fmt + clippy --all-targets + 172 tests green.

### ✅ PHASE C COMPLETE — Gemini provider shipped. All 5 MVP providers (Ollama, OpenAI-compatible, OpenAI, Anthropic, Gemini) implemented & wired.

## Phase D — Rich interactive terminal
- [~] **D1** `DONE (scoped)` — Interactive **status prompt** shipped: each terminal prompt now shows `[profile +included] provider/model approval:X privacy:Y` above `saya> ` (new `session_prompt::status_line`). `handle_line` extracted as a shared helper. Non-TTY/piped path unchanged.
  - reedline (rich history/multi-line editor) was implemented, then **reverted**. Root cause: reedline↔crossterm emit an `ESC[6n` cursor-position query and BLOCK waiting for the terminal to answer; the `active_sigint` PTY test harness (a bare PTY with no terminal emulator) never answers, so reedline errors/hangs. A catch-and-fallback still hung (reedline leaves terminal/signal state altered). reedline works for real users but is incompatible with the PTY test + adds a heavy crossterm build that repeatedly triggered DuckDB rebuilds → ENOSPC on this near-full disk.
  - DECISION: ship the status-prompt (safe, visible UX win, preserves the SIGINT-safety guarantee and all tests). reedline history/editing deferred as a documented follow-up (needs a DSR-answering terminal / a PTY-test strategy). Consistent with the plan's risk clause (escalate/fall back rather than churn).
  - DISK: each reedline add/remove changed the dep graph and re-triggered the ~9 GB DuckDB bundled rebuild on a disk with ~1–2 GB free → repeated ENOSPC; resolved each time with `cargo clean` (→17 GB free). Final clean rebuild + verification running in background.
- [~] **D2** `DEFERRED` — reedline-based history/multi-line editing + slash completion. Deferred with D1's reedline revert (see above); needs a DSR-answering terminal / PTY-test strategy. The status line (D1) shipped.
- [x] **D3** `DONE` — docs/commands.md: interactive status header documented; provider list corrected to all five (ollama/openai/openai_compatible/anthropic/gemini).

### ✅ PHASE D COMPLETE (scoped) — interactive status prompt shipped; rich editor deferred with rationale.

## Phase E — Cleanup
- [x] **E1** `DONE` — `.DS_Store` added to `.gitignore`; CHANGELOG.md updated (multi-DB navigation, Anthropic, Gemini, status prompt, privacy gating). Final verification: fmt clean, clippy `--all-targets -D warnings` clean, **172 tests passed / 0 failed**.

### ✅ PHASE E COMPLETE.

---

## FINAL SUMMARY (all phases green)
- **A — Multi-database navigation:** SHIPPED & proven end-to-end (`connection_registry`, registry-backed tools with a `connection` arg, connections-context system prompt, `--include-profile`/`/include` connect live DBs, integration test).
- **B — Anthropic provider:** SHIPPED (`saya-agent/providers/anthropic*`, wired, policy-gated, tested).
- **C — Gemini provider:** SHIPPED (`saya-agent/providers/gemini*`, wired). All 5 MVP providers implemented; config-accept/runtime-reject footgun eliminated.
- **D — Interactive terminal:** status prompt SHIPPED; reedline rich editor implemented then reverted (bare-PTY DSR incompatibility + disk/ENOSPC), deferred with a documented follow-up path.
- **E — Cleanup:** `.gitignore` + CHANGELOG + docs.
- **Engine:** all feature code written by `agy` (gemini-3.6-flash-high); Claude orchestrated, verified every ticket (fmt+clippy+tests), fixed integration seams, authored the navigation integration test, and made the D scoping call.
- **Also fixed:** a pre-existing flaky test (`duckdb_schema_cache_fallback…` WAL-sidecar TOCTOU race).
- **Baseline → final:** ~190 → 172 fast-path tests, all green (the count differs because some ignored/live tests aren't counted here; net new: registry, provider, privacy, and navigation tests).
- **Not committed** — all changes staged in the working tree for your review (per the plan: commit only on your say-so).

---

## Log
- Plan approved. Beginning Phase A. `agy` non-interactive engine validated (AGY_OK). Baseline: workspace builds, full test suite green (~190 tests).
- Disk was at 100% (target=17G, DuckDB bundled objects=8G); `cargo clean` reclaimed 15G → 18G free. Going forward: builds are incremental; avoid release profile during dev.
- A1: agy created `connection_registry.rs` (119 lines) + tests + lib.rs registration. Code review passed (clean API, no unwrap, `#[allow(dead_code)]` until A2/A4 consume it). fmt+clippy clean, full workspace test green. **A1 DONE.**
- Found + fixed a PRE-EXISTING flaky test `duckdb_schema_cache_fallback_refresh_and_interactive_schema_are_stable` (mvp.rs): TOCTOU race reading a SQLite `-wal`/`-shm` sidecar checkpointed away mid-read. Hardened the read loop to tolerate a vanished sidecar (intent preserved — data folds into the main state file which is also read). Test-only change.
