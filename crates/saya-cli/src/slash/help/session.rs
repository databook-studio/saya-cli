fn command_help_session(name: &str) -> Option<&'static str> {
    match name {
        "clear" => Some("clear — clear the conversation and context. Example: /clear"),
        "compact" => Some(
            "compact — summarise the older turns into a short summary the model replays ahead of \
             the newest verbatim turns. Working memory only: the transcript is unchanged \
             (/export first if you want the full text). Automatic at 95% of the known context \
             window when `[ai] compaction` is `auto` (the default); `manual` keeps only this \
             command, `off` additionally silences the 70% warning — and a failed compaction \
             leaves the conversation exactly as it was. Example: /compact",
        ),
        "history" => Some("history — list saved sessions as text. Example: /history"),
        "sessions" => Some(
            "sessions — browse saved sessions: `/sessions list` prints the saved-session \
                 listing the headless `saya sessions list` shows, `/sessions delete <id>` \
                 deletes one saved session the same way (never prompts; the current session \
                 and any live session are refused), and bare `/sessions` opens a picker in \
                 the TUI. Example: /sessions list",
        ),
        "preferences" => Some(
            "preferences [list|add <text>|edit <id> <text>|forget <id>|forget-all] — explicit, \
             per-worktree guidance you confirmed yourself: never permissions, and stored \
             separately from /remember and the typed contract knowledge. Bare or `list` \
             prints this worktree's preferences, `add <text>` confirms one, `edit <id> \
             <text>` replaces one, `forget <id>` removes one, and `forget-all` removes \
             every preference in this worktree. The worktree is this session's bound \
             workspace root; a session with none bound refuses every form. Never prompts. \
             Example: /preferences add prefer tabs over spaces   or   /preferences list   \
             or   /preferences edit pp-… <new text>   or   /preferences forget pp-…   or   \
             /preferences forget-all",
        ),
        "context" => Some(
            "context — show which project and personal instruction files are loaded into \
             requests: names, whole-file and included sizes, short digests, truncation and \
             redaction counts, plus why any source was skipped (symlink, not a regular file, \
             too large, not UTF-8, unreadable) or omitted for budget, and the effective \
             settings. Never their text: the files carry project guidance only and grant no \
             permissions. Example: /context",
        ),
        "doctor" => Some(
            "doctor — diagnose configuration: secrets resolve? provider endpoint? Example: /doctor",
        ),
        "usage" => Some(
            "usage — show session token usage: input, output, reasoning, cached input, cache creation, and the cache hit rate. The hit rate is Σcached / Σinput across all turns (a ratio of sums, not a mean of per-turn rates). Fields the provider did not report show —; the hit rate shows 'unknown' when no turn reported cached tokens (absent is not zero). Example: /usage",
        ),
        "workspace" => Some(
            "workspace — show the session's bound workspace root, the one tree every file tool \
             and run_program child is contained to. Unbound means the write-shaped tools are \
             hidden and workspace reads refuse. Example: /workspace",
        ),
        "thinking" => Some(
            "thinking [on|off] — toggle display of the model's chain-of-thought in the transcript. Off by default: thinking is verbose (often longer than the answer) and restates database contents in prose. With no argument, toggles; with on/off, sets explicitly. Display only — reasoning is never written to a saved session, and the copy keys (Ctrl+Y, Ctrl+B) leave it out. Selection mode (Ctrl+O) hands the screen to your terminal, so a mouse drag can still copy thinking that is visible. Example: /thinking on",
        ),
        "tasks" => Some(
            "tasks [clear] — show the session's task list: what the model is tracking, one line per task with its state. Bare /tasks shows the list (No tasks are being tracked. when empty); /tasks clear empties it, so the next turn injects no task block. The model's list to write, yours to inspect or clear — no per-task editing lives here. Example: /tasks   or   /tasks clear",
        ),
        "notes" => Some(
            "notes [set <text>|clear] — your own notes attached to this session's saved summary \
             (its continuation record): /notes shows them, /notes set <text> stores them \
             (interior spacing kept), /notes clear removes them. Yours; guidance only — they \
             grant nothing, the model never writes them, and they are separate from /preferences \
             (per worktree) and /remember (database knowledge). A record exists only after \
             /compact: with none, set and clear refuse with that remedy. Text is at most 8192 \
             bytes of plain text (newlines and tabs allowed), and secret-shaped text is refused, \
             never stored. Example: /notes set prefer bounded queries",
        ),
        "resume" => Some("resume <id> — resume a previous session by ID. Example: /resume 12345"),
        "rotate" => Some(
            "rotate — continue in a new, smaller session: saves this session, publishes a \
             successor linked to it (the link is lineage only), and continues there. Requires \
             /compact first: a session with no saved summary has nothing to carry over. The \
             successor keeps the saved summary, your notes and file/check facts, and the \
             not-yet-summarised tail of the conversation; it inherits no grants, approvals, \
             denials, or recovery setting. The parent stays on disk, unchanged and resumable \
             (/resume <old id>). Run it when the session record nears the 4 MiB cap (the \
             store warns at 3 MiB) and compaction alone no longer keeps it small. \
             Example: /rotate",
        ),
        "recovery" => Some(
            "recovery [on|off|list|diff <id>|restore <id>] — record this session's direct \
             file edits so you can undo them (opt-in, off by default). /recovery shows the \
             flag and the quota use; /recovery on turns recording on (from the NEXT file \
             edit) and says what the copies are, where they live, and what recovery never \
             covers; /recovery off stops recording and keeps what was recorded; /recovery \
             list shows the recorded edits with their ids and restore status; /recovery \
             diff <id> previews what restoring would change; /recovery restore <id> puts \
             the file back to its exact pre-edit state — only while the file still is \
             exactly that edit's result, so restore newer edits first. <id> is the full \
             ck-… id from /recovery list or a unique prefix of at least 6 characters \
             after ck-. Never prompts; never a model-callable tool. \
             Example: /recovery   or   /recovery on   or   /recovery off   or   \
             /recovery list   or   /recovery diff ck-123456…   or   /recovery restore ck-123456…",
        ),
        "setup" => Some(
            "setup — run the guided setup from the full-screen TUI: it hands the terminal \
             to the same wizard `saya setup` runs and takes it back when the wizard \
             finishes, is cancelled (Ctrl-D), or fails. Takes no argument. The wizard \
             writes your user configuration. When it finishes, the new configuration \
             is applied right away, as /reload does; settings that cannot change in \
             a running session (data sharing, host commands) are listed as not \
             applied. Headless surfaces (piped REPL, --turn-file) refuse with the \
             pointer to run `saya setup` from your shell. Example: /setup",
        ),
        "reload" => Some(
            "reload — re-read the configuration with the SAME launch options between turns: \
              the report has three sections. `Applied from the next turn` lists settings the \
              agent re-reads per turn (row caps, timeouts, budgets, connections); `Updated \
              for this session` lists the session copies re-seeded now (provider, model, \
              default profile, endpoint) unless you overrode them with /provider, /model or \
              /connect — an override stays yours and is named instead; `Not applied in this \
              session` lists what a running session never changes: data sharing (use /privacy), \
              host commands and session denies (new session), the jobs runner, theme and \
              thinking display. An invalid configuration changes nothing and says the previous \
              one is still in use. Never prompts. Takes no argument. \
              Example: /reload   or   /help reload   (see also: saya config show --explain)",
        ),
        _ => None,
    }
}
