fn command_help_jobs(name: &str) -> Option<&'static str> {
    match name {
        "start" => Some(
            "start [--id <id>] <task> [--write path] [--database-profile name] [--tools name,...] [--plan] [--in-place] [--approve]\nStarts a bounded job in this session's workspace for long or background work; ordinary work is just a message. The task is the whole remaining text, with the flags above removed wherever they appear (put `--` before task text that begins with `--`). The id is generated unless `--id` states one, and the start names the id it chose. With no `--write`, the job may edit the whole workspace (`.`); an explicit `--write` narrows it. Each job needs fresh scope approval; helpers work in the job's one managed working checkout, or in place over a plain folder with `--in-place`.",
        ),
        "follow-up" => Some(
            "follow-up <id> <agent> --expected-completion-sequence <sequence> <instruction> [--approve]\nRuns a bounded new invocation of a completed child in its original owned checkout. The instruction is at most 4096 bytes and grants nothing; --approve renews the job's exact current scope for this invocation.",
        ),
        "job" => Some(
            "job <export|events|review|apply|resume|notes|accept|promote|list|show|cancel|doctor|setup|reconcile|worktrees|rewind> …\nThe durable-job lifecycle verbs, the same grammar `saya job` runs. The agent-shaped controls have their own spellings: /start, /follow-up, /approve, /deny, /message, /pause, /continue and /cancel-child.",
        ),
        "approve" => Some(
            "approve <id> — approve the exact pending job scope this session is showing, opening the write surface for that job. Example: /approve job-a",
        ),
        "deny" => Some(
            "deny <id> — refuse the exact pending job scope this session is showing; the job stops without writing. Example: /deny job-a",
        ),
        "message" => Some(
            "message <id> <agent> <text> — send bounded steering to one child in this session's active job. Example: /message job-a child-a check the null case",
        ),
        "pause" => Some(
            "pause <id> <agent> — pause one active child at its next safe boundary. Example: /pause job-a child-a",
        ),
        "continue" => Some(
            "continue <id> <agent> — continue one paused child in this session's active job. Example: /continue job-a child-a",
        ),
        "cancel-child" => Some(
            "cancel-child <id> <agent> — cancel one child, preserving its siblings. Example: /cancel-child job-a child-a",
        ),
        _ => None,
    }
}
