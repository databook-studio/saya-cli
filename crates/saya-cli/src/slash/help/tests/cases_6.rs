/// K1 pinned the user-visible `/help` listing byte for byte: the catalogue
/// replaces the four hand-maintained tables, and the bytes the user reads must
/// not move. K2 is the one deliberate move since — `/approvals` folded into
/// `/mode` as an alias — so the snapshot below is K1's text with exactly that
/// edit: one line gone, one line's usage and description widened to the six
/// values. Any other difference is a regression.
#[test]
fn help_listing_is_byte_identical() {
    assert_eq!(help_text(), LISTING);
}

const LISTING: &str = r#"Slash commands:

Jobs
  /start [--id <id>] <task> [--write path] [--database-profile name] [--tools name,...] [--plan] [--in-place] [--approve] — Start a job for long or background work; ordinary work is just a message
  /follow-up <id> <agent> --expected-completion-sequence <n> <instruction> [--approve] — Run a bounded new invocation of a completed child in its checkout
  /job <export|events|review|apply|resume|list|show|cancel|doctor|setup|reconcile|worktrees|rewind> … — Operate a durable job's lifecycle: review, apply, resume, list, show, cancel, and the rest
  /approve <id> — Approve the exact pending job scope this session is showing
  /deny <id> — Refuse the exact pending job scope this session is showing
  /message <id> <agent> <text> — Send bounded steering to one child in this session's active job
  /pause <id> <agent> — Pause one active child at its next safe boundary
  /continue <id> <agent> — Continue one paused child in this session's active job
  /cancel-child <id> <agent> — Cancel one child, preserving its siblings

Connections
  /connect <profile> — Replace the active database profile
  /connections — List configured database connections
  /include <profile> — Add a secondary database profile to query scope
  /exclude <profile> — Remove a database profile from query scope

Provider, model & privacy
  /provider [name] — Set or view the AI provider
  /model [name] — Set or view the AI model
  /privacy [on|off] — Enable or disable data sharing privacy
  /mode [ask|accept-edits|plan|bypass|read-only|never] — Set the session's consent mode: ask, accept-edits, plan, bypass, read-only, or never
  /allow <scopes…> — Seed pre-authorised scopes into this session's grant store
  /grants — List the session's granted scopes

Query & data
  /schema [refresh] — Inspect or refresh database schema
  /sql <query> — Run a raw SQL query against the active profile
  /export [--snapshot|--refresh] [--overwrite] <path> — Export the last query's fresh result, or the captured /sql result with --snapshot
  /report [--rows N] [--overwrite] <path> — Write a Markdown report (SQL + provenance) of the captured /sql result
  /investigation save|list|show|run|export|import|delete — Save, list, show, run, export, import, or delete a saved investigation
  /chart [type] [path] — Re-run the last query and render the fresh result as an HTML chart
  /explain [sql] — Explain the given or last SQL statement
  /columns [name,name,… | all] — Choose which columns wide result tables show

Session
  /clear — Clear current session context
  /compact — Summarise older turns to shrink working memory
  /history — List saved sessions as text
  /sessions [list|delete <id>] — Browse saved sessions: list or delete; bare opens a picker in the TUI
  /resume <id> — Resume a saved session by id
  /rotate — Continue in a new, smaller session that keeps your summary and notes (run /compact first)
  /setup — Run the guided setup (full-screen terminal only)
  /reload — Re-read the configuration between turns: applies what is safe now, says what needs a new session. Data sharing and host commands never change in a running session.
  /context — Show the instruction sources loaded into requests, never their text
  /doctor — Diagnose config: secrets, provider endpoint
  /usage — Show session token usage and cache hit rate
  /workspace — Show the session's bound workspace root
  /thinking [on|off] — Toggle display of the model's chain-of-thought
  /tasks [clear] — Show the session's task list, or clear it with /tasks clear
  /notes [set <text>|clear] — Your notes on this session's saved summary: show, set, clear; separate from /preferences and /remember
  /recovery [on|off|list|diff <id>|restore <id>] — Record file edits so you can undo them; list, preview and restore
  /help [command] — Show help for slash commands
  /exit  (alias /quit) — Exit the REPL

Memory
  /contracts [table] — List contracts, or show one object's contract
  /remember <table> <kind> <value…> — Store a confirmed contract claim
  /forget <id> — Tombstone a contract claim so recall excludes it
  /preferences [list|add <text>|edit <id> <text>|forget <id>|forget-all] — Explicit per-worktree guidance you confirmed: list, add, edit, forget; never permissions; separate from /remember
  /queue [limit] — Show pending candidate claims awaiting review
  /confirm <prefix> — Confirm a pending candidate claim by id prefix
  /reject <prefix> — Reject a pending candidate claim by id prefix
  /approve-all [--yes] [limit] — Approve the whole review queue (needs --yes)

Runs
  /run <goal…> --allow <scopes> — Start or operate a headless run from the session
  /runs [id] — List runs, or show one run by id"#;
