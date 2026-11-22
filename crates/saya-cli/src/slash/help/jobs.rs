fn command_help_jobs(name: &str) -> Option<&'static str> {
    match name {
        "start" => Some(
            "start [--id <id>] <task> [--write path] [--database-profile name] [--tools name,...] [--plan] [--in-place] [--approve]\nStarts a bounded job in this session's workspace for long or background work; ordinary work is just a message. The task is the whole remaining text, with the flags above removed wherever they appear (put `--` before task text that begins with `--`). The id is generated unless `--id` states one, and the start names the id it chose. With no `--write`, the job may edit the whole workspace (`.`); an explicit `--write` narrows it. Each job needs fresh scope approval; helpers work in the job's one managed working checkout, or in place over a plain folder with `--in-place`.",
        ),
        "follow-up" => Some(
            "follow-up <id> <agent> --expected-completion-sequence <sequence> <instruction> [--approve] — deprecated alias of /job follow-up, removed in the next release: it runs a bounded new invocation of a completed child in its original owned checkout. The instruction is at most 4096 bytes and grants nothing; --approve renews the job's exact current scope for this invocation. Example: /job follow-up job-a child-a --expected-completion-sequence 3 'check the null case' --approve",
        ),
        "job" => Some(
            "job <export|events|review|apply|resume|notes|accept|promote|list|show|cancel|doctor|setup|reconcile|worktrees|rewind|approve|deny|message|pause|continue|cancel-child|follow-up> …\nThe durable-job lifecycle verbs, the same grammar `saya job` runs, and the child controls. `/start` is the one launch with a spelling of its own; `/approve`, `/deny`, `/message`, `/pause`, `/continue <id> <agent>`, `/cancel-child` and `/follow-up` are `/job`'s verbs under a deprecated top-level spelling for this release.",
        ),
        "approve" => Some(
            "approve <id> — deprecated alias of /job approve, removed in the next release: it approves the exact pending job scope this session is showing, opening the write surface for that job. Example: /job approve job-a",
        ),
        "deny" => Some(
            "deny <id> — deprecated alias of /job deny, removed in the next release: it refuses the exact pending job scope this session is showing; the job stops without writing. Example: /job deny job-a",
        ),
        "message" => Some(
            "message <id> <agent> <text> — deprecated alias of /job message, removed in the next release: it sends bounded steering to one child in this session's active job. Example: /job message job-a child-a check the null case",
        ),
        "pause" => Some(
            "pause <id> <agent> — deprecated alias of /job pause, removed in the next release: it pauses one active child at its next safe boundary. Example: /job pause job-a child-a",
        ),
        "cancel-child" => Some(
            "cancel-child <id> <agent> — deprecated alias of /job cancel-child, removed in the next release: it cancels one child, preserving its siblings. Example: /job cancel-child job-a child-a",
        ),
        _ => None,
    }
}
