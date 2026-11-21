/// K1 pinned the user-visible `/help` listing byte for byte: the catalogue
/// replaces the four hand-maintained tables, and the bytes the user reads must
/// not move. K2 folded `/approvals` into `/mode`. K3 is the one deliberate move
/// since — the knowledge store is one command, `/contracts <verb>`: the
/// `/contracts` row widens to its eight verbs and the four rows `/queue`,
/// `/confirm`, `/reject`, `/approve-all` are gone (they are catalogue aliases
/// now, folded out of the listing). `/undo` and `/rewind` then join the Session
/// section after `/clear`, where the run's checkpoints are undone. K5 folds
/// `/history` into `/sessions`, `/rotate` into `/compact [--new-session]`, and
/// `/usage`, `/workspace`, `/context` into the one `/status` row (all five are
/// catalogue aliases now). Any other difference is a regression.
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
  /permissions — One read-only view: the mode, the grant store, the deny list, the host lane and data sharing

Query & data
  /schema [refresh] — Inspect or refresh database schema
  /sql <query> — Run a raw SQL query against the active profile
  /export [--snapshot|--refresh] [--overwrite] [--rows N] [--format csv|json|md] <path> — Export the last query's fresh result, or the captured /sql result with --snapshot; a .md path writes today's Markdown report
  /investigation save|list|show|run|export|import|delete — Save, list, show, run, export, import, or delete a saved investigation
  /chart [type] [path] — Re-run the last query and render the fresh result as an HTML chart
  /explain [sql] — Explain the given or last SQL statement
  /columns [name,name,… | all] — Choose which columns wide result tables show

Session
  /clear — Clear current session context
  /undo — Undo the last turn's file edits: restore the run's checkpoint taken before that turn
  /rewind [<id>] — List the run's checkpoints, newest first, or restore the one you name
  /compact [--new-session] — Summarise older turns to shrink working memory; --new-session continues in a smaller successor
  /sessions [list|delete <id>] — Browse saved sessions: list or delete; bare opens a picker in the TUI
  /resume <id> — Resume a saved session by id
  /setup — Run the guided setup (full-screen terminal only)
  /reload — Re-read the configuration between turns: applies what is safe now, says what needs a new session. Data sharing and host commands never change in a running session.
  /doctor — Diagnose config: secrets, provider endpoint
  /status — One panel: the session, the model, the workspace, the instruction sources and the limits in force. It only reads; /doctor is the command that checks
  /thinking [on|off] — Toggle display of the model's chain-of-thought
  /tasks [clear] — Show the session's task list, or clear it with /tasks clear
  /notes [set <text>|clear] — Your notes on this session's saved summary: show, set, clear; separate from /preferences and /remember
  /recovery [on|off|list|diff <id>|restore <id>] — Record file edits so you can undo them; list, preview and restore
  /help [command] — Show help for slash commands
  /exit  (alias /quit) — Exit the REPL

Memory
  /contracts <list|show|queue|confirm|reject|confirm-all|export|import> — The knowledge store: list every contract, show one object's contract, review and decide the queue, and export or import the context
  /remember <table> <kind> <value…> — Store a confirmed contract claim
  /forget <id> — Tombstone a contract claim so recall excludes it
  /preferences [list|add <text>|edit <id> <text>|forget <id>|forget-all] — Explicit per-worktree guidance you confirmed: list, add, edit, forget; never permissions; separate from /remember

Runs
  /run <goal…> --allow <scopes> — Start or operate a headless run from the session
  /runs [id] — List runs, or show one run by id"#;
