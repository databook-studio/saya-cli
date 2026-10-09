fn command_help_connection_provider(name: &str) -> Option<&'static str> {
    match name {
        "connect" => {
            Some("connect <profile> — set the active database profile. Example: /connect prod")
        }
        "connections" => Some(
            "connections — list configured database connection profiles. Example: /connections",
        ),
        "include" => Some(
            "include <profile> — include an additional database profile. Example: /include staging",
        ),
        "exclude" => {
            Some("exclude <profile> — exclude a database profile. Example: /exclude staging")
        }
        "provider" => {
            Some("provider [name] — view or set the AI provider. Example: /provider anthropic")
        }
        "model" => Some("model [name] — view or set the AI model. Example: /model gpt-4o"),
        "privacy" => {
            Some("privacy [on|off] — view or toggle cloud data sharing. Example: /privacy off")
        }
        "approvals" => Some(
            "approvals [ask|read-only|never|bypass] — view or set tool execution approval policy. \
             `bypass` runs every call without asking (a typed, global consent given at the flag or \
             this command); every structural guard still applies. Example: /approvals ask",
        ),
        "mode" => Some(
            "mode [plan|build] — view or set the agent's task posture. `plan` investigates \
             read-only: write-shaped tools are hidden from the model and refuse before the \
             approval match runs, so Plan still denies writes under `bypass` (bypass \
             auto-allows reads; Plan refuses writes). What Plan guarantees is that a write \
             attempt fails — hidden definitions, engine denial, derived write permits off; \
             ending with a plan rather than a half-done edit is what the model is asked to \
             do, not what Plan guarantees. Plan is the task posture — what the \
             agent may do — while `read-only` is the consent posture — what an approval \
             answers. Example: /mode plan",
        ),
        "schema" => Some(
            "schema [refresh] — display or refresh database schema context. Example: /schema refresh",
        ),
        "sql" => Some(
            "sql <query> — execute a raw SQL query directly. Example: /sql SELECT * FROM users LIMIT 10;",
        ),
        _ => None,
    }
}
