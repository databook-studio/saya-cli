fn command_help_query(name: &str) -> Option<&'static str> {
    match name {
        "export" => Some(
            "export [--snapshot|--refresh] [--overwrite] [--rows N] [--format csv|json|md] <path> — \
             write the result to a file. The extension or --format selects it: a .csv or .json \
             path writes the rows; a .md path (or --format md) writes the shareable Markdown \
             report. The default (and --refresh) re-runs the last query and exports that fresh \
             read — the file reflects the fresh result, not the displayed table (/columns, \
             scroll, and folds do not apply). --snapshot exports the result you already have: \
             the latest /sql capture, held for this session only, with no query at all. A \
             Markdown report is always written from that capture, provenance included: connection \
             label, submitted-SQL hash, execution id, times, row counts, truncation, scope — \
             with the rows section reading that rows are omitted unless --rows N (at most 100) \
             adds a table of the first N. Cell values are neutralised, so spreadsheet formulas, \
             links, HTML, and control characters cannot be carried into the file — review a \
             report before sharing. An existing destination needs --overwrite. \
             Example: /export results.csv or /export notes.md --rows 20",
        ),
        "report" => Some(
            "report [--rows N] [--overwrite] <path> — deprecated alias for /export --format md \
             (removed next release): the same operation, the same bytes. It writes the \
             shareable Markdown report of the result you already inspected — the latest /sql \
             capture, held for this session only — and never queries a database. By default \
             the report carries the exact SQL and provenance only; pass --rows N (at most 100) \
             to add a table of the first N captured rows. Use /export for data files. \
             Example: /report notes.md   or   /export notes.md --rows 20",
        ),
        "investigation" => Some(
            "investigation <save|list|show|run|export|import|delete> — manage saved \
             investigations: portable JSON documents holding one exact, bounded, \
             read-only SQL query and nothing else. `/investigation save <name>` \
             saves the latest successful, concrete query on the connection that \
             actually ran it; pass --sql <SQL> (or --file <PATH>) to save \
             different SQL, and --connection <PROFILE> to save against another \
             profile. Saving validates the SQL with the same read-only gate \
             execution uses but never runs it; the exact SQL is stored verbatim, \
             so review it before sharing. `run <id>` replays one on its explicit \
             connection — it runs in the background like a /sql query, so the \
             transcript keeps working while it runs; Esc detaches it (the query \
             may still be running on the server). `list`, `show`, `delete`, \
             `export <id> <path>`, and \
             `import <path>` are the same operations the `saya investigation` \
             commands provide. Example: /investigation save recent orders   or   \
             /investigation list",
        ),
        "investigations" => Some(
            "investigations — open a searchable picker over the saved investigations \
             (type to filter · Enter shows the chosen one · r runs it · Esc closes); \
             `/investigations --list` or `/investigation list` prints the text list. \
             Example: /investigations",
        ),
        "chart" => Some(
            "chart [type] [path] — re-run the last query and render the fresh result as an interactive HTML chart, then open it. The chart reflects that fresh read, not the displayed table. type: bar|line|area|pie|doughnut|scatter (default auto)",
        ),
        "explain" => Some(
            "explain [sql] — show the query plan (EXPLAIN) for the given SQL, or the last query if omitted",
        ),
        "columns" => Some(
            "columns [name,name,… | all] — choose which columns wide result tables show in the TUI. Names match column headers (case-insensitive); unmatched names are ignored, and a filter that matches nothing falls back to all columns. /columns or /columns all resets. The full table is still copied by Ctrl+B; this only changes what is painted. Example: /columns id, total   or   /columns all",
        ),
        _ => None,
    }
}
