pub fn parse_slash_command(input: &str) -> Result<Option<SlashCommand>, SlashParseError> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return Ok(None);
    }
    let mut parts = trimmed[1..].split_whitespace();
    let name = parts.next().unwrap_or_default();
    let arg = parts.collect::<Vec<_>>().join(" ");
    let required = || {
        (!arg.is_empty())
            .then_some(arg.clone())
            .ok_or_else(|| SlashParseError("command requires an argument".into()))
    };
    let command = match name {
        // The retired `/coding` spelling is still recognized, so the answer
        // names the spellings that work instead of "unknown command".
        "coding" => {
            return Err(SlashParseError(
                "`/coding` was replaced: `/start` starts a job, `/job <verb>` runs the \
                 lifecycle verbs and the child controls, and bare `/continue` continues a \
                 paused turn"
                    .into(),
            ));
        }
        "job" => job::parse_job(trimmed.strip_prefix("/job").unwrap_or("").trim_start())?,
        "start" => job::parse_start(trimmed.strip_prefix("/start").unwrap_or("").trim_start())?,
        "follow-up" => {
            job::parse_follow_up(trimmed.strip_prefix("/follow-up").unwrap_or("").trim_start())?
        }
        "approve" | "deny" | "message" | "pause" | "cancel-child" => {
            // The tail keeps its interior whitespace and quotes: a message's
            // text is free text, and `tokenize_tail` is the one splitter.
            let tail = trimmed
                .strip_prefix(&format!("/{name}"))
                .unwrap_or("")
                .trim_start();
            job::parse_control(name, tail)?
        }
        // `/continue` is two spellings of two operations: bare continues the
        // paused turn, and `<id> <agent>` is `/job continue`'s one-release
        // top-level spelling.
        "continue" => {
            let tail = trimmed
                .strip_prefix("/continue")
                .unwrap_or("")
                .trim_start();
            job::parse_continue(tail)?
        }
        "connect" => SlashCommand::Connect(required()?),
        "connections" => SlashCommand::Connections,
        "include" => SlashCommand::Include(required()?),
        "exclude" => SlashCommand::Exclude(required()?),
        "provider" => SlashCommand::Provider((!arg.is_empty()).then_some(arg)),
        "model" => SlashCommand::Model((!arg.is_empty()).then_some(arg)),
        "privacy" => SlashCommand::Privacy(parse_bool(&arg)?),
        // `/approvals` is `/mode`'s one-release alias, not a second command:
        // one variant, one grammar, one switch.
        "approvals" => SlashCommand::Mode(ModeRequest::alias(parse_mode(&arg)?)),
        "mode" => SlashCommand::Mode(ModeRequest::mode(parse_mode(&arg)?)),
        "schema" => SlashCommand::Schema(arg == "refresh"),
        "sql" => {
            let query = trimmed.strip_prefix("/sql").unwrap_or("").trim();
            if query.is_empty() {
                return Err(SlashParseError("sql requires a query".into()));
            }
            SlashCommand::Sql(query.to_string())
        }
        "export" => {
            let tail = trimmed.strip_prefix("/export").unwrap_or("").trim_start();
            SlashCommand::Export(parse_export(tail)?)
        }
        // `/report` is `/export --format md`'s one-release alias, not a
        // second command: one variant, one writer, one document.
        "report" => {
            let tail = trimmed.strip_prefix("/report").unwrap_or("").trim_start();
            SlashCommand::Export(parse_report(tail)?)
        }
        "chart" => SlashCommand::Chart(arg.trim().to_string()),
        "explain" => SlashCommand::Explain(arg.trim().to_string()),
        "clear" => SlashCommand::Clear,
        "compact" => {
            // Bare compacts this session; `--new-session` compacts and
            // continues in a successor, which is the one operation
            // `/rotate` runs — its one-release alias states that spelling.
            let tail = trimmed
                .strip_prefix("/compact")
                .unwrap_or("")
                .trim_start();
            SlashCommand::Compact(parse_compact(tail)?)
        }
        // `/history` is `/sessions`'s one-release alias, not a second
        // command: one variant, one grammar, one listing.
        "history" => SlashCommand::Sessions(SessionsRequest::history()),
        "sessions" => {
            // Bare `/sessions` keeps today's meaning (REPL text history,
            // TUI picker). `list` and `delete <id>` are the shared
            // saved-session operations; anything else is a usage error
            // naming every valid form, so a typo cannot read as a silent
            // show.
            match arg.as_str() {
                "" => SlashCommand::Sessions(SessionsRequest::default()),
                "list" => SlashCommand::SessionsList,
                other => {
                    let (head, rest) = other.split_once(' ').unwrap_or((other, ""));
                    if head != "delete" {
                        return Err(SlashParseError(format!(
                            "unknown /sessions subcommand: {other} (usage: /sessions, \
                             /sessions list, /sessions delete <id>)"
                        )));
                    }
                    let id = rest.trim();
                    if id.is_empty() {
                        return Err(SlashParseError(
                            "sessions delete requires a session id: /sessions delete <id>".into(),
                        ));
                    }
                    if id.split_whitespace().count() > 1 {
                        return Err(SlashParseError(
                            "sessions delete takes exactly one session id: \
                             /sessions delete <id>"
                                .into(),
                        ));
                    }
                    SlashCommand::SessionsDelete(id.to_string())
                }
            }
        }
        // `/resume <id>` opens a saved session; bare `/resume` opens the saved
        // sessions to choose from — the picker on the full-screen surface, the
        // recent ids on the line front.
        "resume" => SlashCommand::Resume(arg),
        "rotate" => {
            // `/rotate` takes no argument, and it is `/compact --new-session`'s
            // one-release alias: the same variant, carrying the spelling so
            // its note rides the one operation's answer.
            if !arg.is_empty() {
                return Err(SlashParseError(
                    "rotate takes no argument: /rotate (alias of /compact --new-session)".into(),
                ));
            }
            SlashCommand::Compact(CompactRequest::rotate())
        }
        "setup" => {
            // `/setup` takes no argument: a trailing word is a typo, and the
            // pinned error says so instead of reading it as one.
            if !arg.is_empty() {
                return Err(SlashParseError("setup takes no argument: /setup".into()));
            }
            SlashCommand::Setup
        }
        "reload" => {
            // `/reload` takes no argument: it re-reads the same launch
            // inputs, so anything else typed after it is a mistake worth
            // naming, never a silent ignore.
            if !arg.is_empty() {
                return Err(SlashParseError("reload takes no argument: /reload".into()));
            }
            SlashCommand::Reload
        }
        "preferences" => {
            // `/preferences` is free text on the add and edit forms, so the
            // tail keeps its interior whitespace verbatim — the `/sql`
            // idiom, not the whitespace-collapsing default — and the
            // sub-word split happens once, here, mapping onto the same
            // `PreferencesCommand` the headless parser produces.
            let tail = trimmed.strip_prefix("/preferences").unwrap_or("").trim();
            SlashCommand::Preferences(parse_preferences(tail)?)
        }
        "notes" => {
            // `/notes` is free text on the set form, so the tail keeps its
            // interior whitespace verbatim — the `/sql` idiom, like
            // `/preferences` — and the sub-word split happens once, here,
            // mapping onto the same `NotesCommand` the shared `run_notes`
            // operation runs on both surfaces.
            let tail = trimmed.strip_prefix("/notes").unwrap_or("").trim();
            SlashCommand::Notes(parse_notes(tail)?)
        }
        "recovery" => {
            // `/recovery` carries no free text: the sub-word split happens
            // once, here, mapping onto the same `RecoveryCommand` the
            // shared `run_recovery` operation runs on every surface.
            let tail = trimmed.strip_prefix("/recovery").unwrap_or("").trim();
            SlashCommand::Recovery(parse_recovery(tail)?)
        }
        "columns" => SlashCommand::Columns((!arg.is_empty()).then_some(arg)),
        "undo" => {
            // `/undo` takes no argument: the run's checkpoints are named with
            // `/rewind`, and a trailing word here would be a silent ignore.
            let tail = trimmed.strip_prefix("/undo").unwrap_or("").trim();
            parse_undo(tail)?;
            SlashCommand::Undo
        }
        "rewind" => {
            // `/rewind [<id>]`: bare is the listing, one word names the
            // checkpoint restored. The id is resolved against the listing by
            // the operation, so a typo is refused by name.
            let tail = trimmed
                .strip_prefix("/rewind")
                .unwrap_or("")
                .trim_start();
            SlashCommand::Rewind(parse_rewind(tail)?)
        }
        "doctor" => SlashCommand::Doctor,
        "status" => SlashCommand::Status,
        "usage" => {
            // Bare shows the panel; `--limit <usd>` also states this
            // session's ceiling. The tail is split once, here.
            let tail = trimmed.strip_prefix("/usage").unwrap_or("").trim();
            SlashCommand::Usage(parse_usage(tail)?)
        }
        "thinking" => SlashCommand::Thinking(parse_bool(&arg)?),
        "workspace" => SlashCommand::Workspace,
        "contracts" | "contract" | "remember" | "forget" | "queue" | "confirm" | "reject"
        | "approve-all" => {
            // The contract slash adapters: translate to the same
            // `ContractsCommand` the headless parser produces and hand it to
            // the shared dispatcher. No second parsing or DTO mapping lives here.
            // `confirm`/`reject` translate to `ContractsCommand::Decide`.
            return contracts::parse_contract_command(name, &arg)
                .map(|maybe| maybe.map(SlashCommand::Contracts));
        }
        "investigation" | "investigations" => {
            // The saved-investigation slash adapter: translate to the same
            // `InvestigationCommand` the headless parser produces and hand it
            // to the shared `run_investigation` operation. `/investigations`
            // is `list`. The tail is verbatim — a flag value runs to the next
            // known flag, so SQL keeps its interior whitespace.
            let tail = trimmed
                .strip_prefix(&format!("/{name}"))
                .unwrap_or("")
                .trim_start();
            return investigation::parse_investigation_command(name, tail)
                .map(|command| Some(SlashCommand::Investigation(command)));
        }
        "run" => {
            // `/run` and `/run cancel <id>`. The subcommand word is matched
            // first, exactly as the headless `saya run` CLI disambiguates:
            // `saya run cancel <id>` is the Cancel subcommand, so a slash
            // goal beginning with "cancel" needs the same treatment the
            // headless grammar already gives it. Everything else is the
            // nested-run tail, handed to the child verbatim.
            let tail = arg.trim();
            if tail.is_empty() {
                return Err(SlashParseError(
                    "run requires a goal and --allow scopes, e.g. \
                     /run survey the data --allow workspace-write"
                        .into(),
                ));
            }
            let (head, rest) = tail.split_once(' ').unwrap_or((tail, ""));
            if head == "cancel" {
                let run_id = rest.trim();
                if run_id.is_empty() {
                    return Err(SlashParseError(
                        "run cancel requires a run id: /run cancel <id>".into(),
                    ));
                }
                SlashCommand::RunCancel(run_id.to_string())
            } else {
                SlashCommand::Run(tail.to_string())
            }
        }
        "runs" => SlashCommand::Runs((!arg.is_empty()).then_some(arg)),
        "allow" => {
            // `/allow <scopes…>` seeds the session's grant store; `/allow`
            // alone is not a listing — `/permissions` is the view, and the
            // error says where to find it.
            if arg.is_empty() {
                return Err(SlashParseError(
                    "allow requires scopes to seed, e.g. /allow sql:analytics \
                     (or `/allow none` for the empty approval); /allow alone is \
                     not a listing — use /permissions to see the grants"
                        .into(),
                ));
            }
            SlashCommand::Allow(arg.split_whitespace().map(str::to_string).collect())
        }
        "context" => SlashCommand::Context,
        // `/grants` is `/permissions`'s one-release alias, not a second
        // command: one variant, one view, one rendering.
        "grants" => SlashCommand::Permissions(PermissionsRequest::grants()),
        "permissions" => SlashCommand::Permissions(PermissionsRequest::permissions()),
        "help" => SlashCommand::Help((!arg.is_empty()).then_some(arg)),
        "tasks" => match arg.as_str() {
            "" => SlashCommand::Tasks(None),
            "clear" => SlashCommand::Tasks(Some("clear".into())),
            other => {
                return Err(SlashParseError(format!(
                    "unknown /tasks subcommand: {other} (usage: /tasks or /tasks clear)"
                )));
            }
        },
        "exit" | "quit" => SlashCommand::Exit,
        other => {
            let msg = match registry::closest_command(other) {
                Some(sugg) => format!("unknown command: /{other} (did you mean /{sugg}?)"),
                None => format!("unknown command: /{other}"),
            };
            return Err(SlashParseError(msg));
        }
    };
    Ok(Some(command))
}
