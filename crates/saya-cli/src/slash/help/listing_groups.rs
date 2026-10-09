/// Listing-only presentation: the group heading, command name, and usage form
/// for each shown command, in display order. The description text is NOT here
/// — it comes from [`COMMAND_DESCRIPTIONS`] via [`description_for`], so there is
/// one copy. The alias spellings `/contract` and `/quit` are folded under
/// `/contracts` and `/exit` (the listing must not show `/contract <table>`), so
/// this lists the canonical form of each command, not every alias.
const LISTING_GROUPS: &[(&str, &[(&str, &str)])] = &[
    (
        "Connections",
        &[
            ("connect", "/connect <profile>"),
            ("connections", "/connections"),
            ("include", "/include <profile>"),
            ("exclude", "/exclude <profile>"),
        ],
    ),
    (
        "Provider, model & privacy",
        &[
            ("provider", "/provider [name]"),
            ("model", "/model [name]"),
            ("privacy", "/privacy [on|off]"),
            ("approvals", "/approvals [ask|read-only|never|bypass]"),
            ("mode", "/mode [plan|build]"),
            ("allow", "/allow <scopes…>"),
            ("grants", "/grants"),
        ],
    ),
    (
        "Query & data",
        &[
            ("schema", "/schema [refresh]"),
            ("sql", "/sql <query>"),
            (
                "export",
                "/export [--snapshot|--refresh] [--overwrite] <path>",
            ),
            ("report", "/report [--rows N] [--overwrite] <path>"),
            (
                "investigation",
                "/investigation save|list|show|run|export|import|delete",
            ),
            ("chart", "/chart [type] [path]"),
            ("explain", "/explain [sql]"),
            ("columns", "/columns [name,name,… | all]"),
        ],
    ),
    (
        "Session",
        &[
            ("clear", "/clear"),
            ("compact", "/compact"),
            ("history", "/history"),
            ("sessions", "/sessions [list|delete <id>]"),
            ("resume", "/resume <id>"),
            ("rotate", "/rotate"),
            ("setup", "/setup"),
            ("reload", "/reload"),
            ("context", "/context"),
            ("doctor", "/doctor"),
            ("usage", "/usage"),
            ("workspace", "/workspace"),
            ("thinking", "/thinking [on|off]"),
            ("tasks", "/tasks [clear]"),
            ("notes", "/notes [set <text>|clear]"),
            ("recovery", "/recovery [on|off|list|diff <id>|restore <id>]"),
            ("help", "/help [command]"),
            ("exit", "/exit  (alias /quit)"),
        ],
    ),
    (
        "Memory",
        &[
            ("contracts", "/contracts [table]"),
            ("remember", "/remember <table> <kind> <value…>"),
            ("forget", "/forget <id>"),
            (
                "preferences",
                "/preferences [list|add <text>|edit <id> <text>|forget <id>|forget-all]",
            ),
            ("queue", "/queue [limit]"),
            ("confirm", "/confirm <prefix>"),
            ("reject", "/reject <prefix>"),
            ("approve-all", "/approve-all [--yes] [limit]"),
        ],
    ),
    (
        "Runs",
        &[
            ("run", "/run <goal…> --allow <scopes>"),
            ("runs", "/runs [id]"),
        ],
    ),
];
