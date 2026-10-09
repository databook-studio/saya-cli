fn command_help_memory_runs(name: &str) -> Option<&'static str> {
    match name {
        "contracts" => Some(
            "contracts [catalog.schema.object] — list every recalled contract for the active profile, or show one object's contract when you name it. Example: /contracts   or   /contracts analytics.public.orders",
        ),
        // `/contract` is a silent alias of the merged `/contracts` command,
        // so `/help contract` returns the same help rather than "no help".
        "contract" => Some(
            "contract [catalog.schema.object] — alias for /contracts: list every recalled contract, or show one object's contract when you name it. Example: /contract analytics.public.orders",
        ),
        "remember" => Some(
            "remember <catalog.schema.object> <kind> <value…> [because <reason…>] — store a confirmed claim. Kinds: description, alias, grain, time-column, column-description <column> <value…>, column-role <column> <role>. The optional `because <reason…>` (directive kinds only) records why the claim holds. Example: /remember analytics.public.orders time-column created_at because orders complete on return",
        ),
        "forget" => Some(
            "forget <claim-id> — tombstone a claim so recall excludes it. Example: /forget abc-123",
        ),
        "queue" => {
            Some("queue [limit] — list candidate claims awaiting review. Example: /queue 20")
        }
        "confirm" => Some(
            "confirm <claim-id-prefix> — confirm the claim named by its short id prefix (the ki-xxxx form /contracts shows). Example: /confirm ki-a86a3f",
        ),
        "reject" => Some(
            "reject <claim-id-prefix> — reject the claim named by its short id prefix. Example: /reject ki-a86a3f",
        ),
        "approve-all" => Some(
            "approve-all [--yes] [limit] — approve every candidate in the review queue: the same set /queue shows. Each candidate still gets the per-item validation /confirm applies, so some may be refused; every approval and every refusal is reported by id. Without --yes the queue is printed and nothing is approved. Example: /approve-all --yes",
        ),
        "run" => Some(
            "run <goal…> --allow <scopes> [--budget k=v…] — start a headless run from the session: the nested `saya run` streams its events and lands where the headless command lands (completed 0, paused 6 — resume it with /run resume <id> — cancelled 130). `none` states a read-only run with no capabilities; the wired scopes are `workspace-write`, `scratch`, `fetch:<scheme>+<host>`, `runner:<program>`, `interpreter:<program>` (a shell or interpreter — a program that can spawn arbitrary children, so naming it is the only grant), and `sql:<connection>` (seeds the run's decider: under `--approval-mode ask` the read-shaped SQL tools' calls naming that connection run without asking). `command:<program>` is refused here: a run is unattended and this scope names unconfined host execution. `endpoint:<role>=<endpoint>` is refused here: per-step endpoint roles are not bound. Prefix the tail with `--seed-grants` to seed the child's `--allow` from this session's grant store — only tokens a run accepts are forwarded, and the rest are named, not silently dropped. `/run cancel <id>` records a run cancelled the same way `saya run cancel` does; a run with a live holder refuses. Example: /run survey the data --allow workspace-write",
        ),
        "runs" => Some(
            "runs [id] — list every run, most recent first, or show one run's status, goal, scopes, pause reason, and deliverables when you name its id. Same rendering as `saya run list|show`. Example: /runs   or   /runs r1726820000000-1234",
        ),
        "allow" => Some(
            "allow <scopes…> — seed pre-authorized scopes into this session's grant store: a granted scope pre-answers the asks it names until the session ends (grants die with the session and are never persisted; a resumed session starts empty). The grammar is the same one `--allow` parses, judged for the session surface: the wired scopes are `workspace-write`, `scratch`, `fetch:<scheme>+<host>`, `runner:<program>`, `interpreter:<program>`, `command:<program>` (a host program — unsandboxed, as your user with your filesystem and network — gated on the lane, which composes wherever a workspace root binds; refused here when no root is bound), and `sql:<connection>` (one SQL grant covers the read-shaped SQL family — bounded_sql_query, result_shape, column_health, join_check — against that connection). `endpoint:<role>=<endpoint>` is refused here: a session binds no per-step endpoint roles. A scope naming a denied program refuses here: a grant cannot override the deny list, and the deny list bounds only the program named in the ask — allowed programs may still invoke it. `none` states the empty approval and must stand alone: it seeds nothing, and it is not a revoke — the store keeps whatever it already holds. /allow only adds. Example: /allow sql:analytics   or   /allow sql:analytics runner:bench",
        ),
        "grants" => Some(
            "grants — list this session's grant store verbatim: one granted scope per line, sorted, under a header stating the lifetime, with a count. The words are the record, and they are the same words /allow seeded and the prompts offered. Example: /grants",
        ),
        "help" => Some(
            "help [command] — display general help or detailed usage for a command. Example: /help connect",
        ),
        "exit" | "quit" => Some("exit — exit the interactive CLI session. Example: /exit"),
        _ => None,
    }
}
