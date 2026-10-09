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
        "connect" => SlashCommand::Connect(required()?),
        "connections" => SlashCommand::Connections,
        "include" => SlashCommand::Include(required()?),
        "exclude" => SlashCommand::Exclude(required()?),
        "provider" => SlashCommand::Provider((!arg.is_empty()).then_some(arg)),
        "model" => SlashCommand::Model((!arg.is_empty()).then_some(arg)),
        "privacy" => SlashCommand::Privacy(parse_bool(&arg)?),
        "approvals" => SlashCommand::Approvals(parse_approval(&arg)?),
        "mode" => SlashCommand::Mode(parse_mode(&arg)?),
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
        "report" => {
            let tail = trimmed.strip_prefix("/report").unwrap_or("").trim_start();
            SlashCommand::Report(parse_report(tail)?)
        }
        "chart" => SlashCommand::Chart(arg.trim().to_string()),
        "explain" => SlashCommand::Explain(arg.trim().to_string()),
        "clear" => SlashCommand::Clear,
        "compact" => SlashCommand::Compact,
        "history" => SlashCommand::History,
        "sessions" => {
            // Bare `/sessions` keeps today's meaning (REPL text history,
            // TUI picker). `list` and `delete <id>` are the shared
            // saved-session operations; anything else is a usage error
            // naming every valid form, so a typo cannot read as a silent
            // show.
            match arg.as_str() {
                "" => SlashCommand::Sessions,
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
        "resume" => SlashCommand::Resume(required()?),
        "rotate" => {
            // `/rotate` takes no argument: a trailing word is a mistake
            // worth naming, never a silent ignore.
            if !arg.is_empty() {
                return Err(SlashParseError("rotate takes no argument: /rotate".into()));
            }
            SlashCommand::Rotate
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
        "doctor" => SlashCommand::Doctor,
        "usage" => SlashCommand::Usage,
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
            // alone is not a listing — `/grants` is the listing, and the
            // error says where to find it.
            if arg.is_empty() {
                return Err(SlashParseError(
                    "allow requires scopes to seed, e.g. /allow sql:analytics \
                     (or `/allow none` for the empty approval); /allow alone is \
                     not a listing — use /grants to list"
                        .into(),
                ));
            }
            SlashCommand::Allow(arg.split_whitespace().map(str::to_string).collect())
        }
        "context" => SlashCommand::Context,
        "grants" => SlashCommand::Grants,
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
