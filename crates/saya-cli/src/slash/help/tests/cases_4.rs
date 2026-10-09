/// `/tasks` is wired at every hand-maintained touchpoint the
/// cli-application standard names: the registry, the description table
/// (the popup's single source), the `/help` listing, and the per-command
/// help. The detailed entry states the ownership split: the model's list
/// to write, the user's to inspect or clear — no per-task editing here.
#[test]
fn tasks_is_registered_listed_and_described() {
    assert!(
        registry::KNOWN_COMMANDS.contains(&"tasks"),
        "tasks is registered"
    );
    assert!(
        description_for("tasks").is_some(),
        "tasks has a popup description"
    );
    let listing = help_text();
    assert!(
        listing.contains("/tasks [clear]"),
        "the listing shows the /tasks usage: {listing}"
    );
    let help = command_help("tasks").expect("tasks has per-command help");
    assert!(
        help.contains("/tasks clear"),
        "the /tasks help names the clear form: {help}"
    );
    assert!(
        help.contains("No tasks are being tracked."),
        "the /tasks help names the empty-list words: {help}"
    );
}

/// `/preferences` is wired at every hand-maintained touchpoint the
/// cli-application standard names: the registry, the description table
/// (the popup's single source), the `/help` listing, and the per-command
/// help. The entry states the contract — explicit, per-worktree
/// guidance; never permissions; separate from /remember — with one
/// example per subcommand.
#[test]
fn preferences_is_registered_listed_and_described() {
    assert!(
        registry::KNOWN_COMMANDS.contains(&"preferences"),
        "preferences is registered"
    );
    assert!(
        description_for("preferences").is_some(),
        "preferences has a popup description"
    );
    let listing = help_text();
    assert!(
        listing.contains("/preferences [list|add <text>|edit <id> <text>|forget <id>|forget-all]"),
        "the listing shows the /preferences usage: {listing}"
    );
    let help = command_help("preferences").expect("preferences has per-command help");
    assert!(
        help.contains("never permissions"),
        "the /preferences help states the never-permissions contract: {help}"
    );
    assert!(
        help.contains("separately from /remember"),
        "the /preferences help states the /remember separation: {help}"
    );
    for form in [
        "/preferences list",
        "/preferences add",
        "/preferences edit",
        "/preferences forget pp-…",
        "/preferences forget-all",
    ] {
        assert!(
            help.contains(form),
            "the /preferences help shows an example for {form}: {help}"
        );
    }
}

/// `/mode` is wired at every hand-maintained touchpoint the
/// cli-application standard names: the registry, the description table
/// (the popup's single source), the `/help` listing, and the per-command
/// help. The detailed entry states the composition honestly: under
/// `bypass`, Plan still denies writes (bypass auto-allows reads, Plan
/// refuses writes), and Plan is a task posture while `read-only` is a
/// consent posture. The entry says both halves — the enforced half
/// (hidden definitions, refusal before the approval match, derived write
/// permits off) and the asked-of-the-model half (ending with a plan) —
/// and never claims Plan guarantees the model's behaviour.
#[test]
fn mode_is_registered_listed_described_and_honest() {
    assert!(
        registry::KNOWN_COMMANDS.contains(&"mode"),
        "mode is registered"
    );
    assert!(
        description_for("mode").is_some(),
        "mode has a popup description"
    );
    let listing = help_text();
    assert!(
        listing.contains("/mode [plan|build]"),
        "the listing shows the /mode usage: {listing}"
    );
    let help = command_help("mode").expect("mode has per-command help");
    assert!(
        help.contains(PLAN_BYPASS_SENTENCE),
        "the /mode help states the bypass composition verbatim: {help}"
    );
    assert!(
        help.contains("task posture") && help.contains("consent posture"),
        "the /mode help separates Plan from read-only: {help}"
    );
    assert!(
        help.contains("hidden definitions") && help.contains("ending with a plan"),
        "the /mode help says both halves — enforced denial and asked-of-the-model plan: {help}"
    );
    assert!(
        !help.contains("guarantees the model"),
        "the /mode help must never claim Plan guarantees the model's behaviour: {help}"
    );
}

/// `/help setup` must describe the shipped behaviour: since the automatic
/// reload, a finished `/setup` applies the new configuration immediately,
/// the same way `/reload` does — the PTY test
/// `first_run_setup_applies_immediately_without_a_restart` pins that and
/// `docs/commands.md` documents it. Help that still says the new
/// configuration "applies to your NEXT saya session" is wrong exactly
/// where most people read it.
#[test]
fn setup_help_describes_the_immediate_apply() {
    let help = command_help("setup").expect("setup has per-command help");
    assert!(
        help.contains("applied right away"),
        "the /setup help must say the new configuration is applied right away: {help}"
    );
    assert!(
        !help.contains("NEXT"),
        "the /setup help must not claim the configuration waits for the NEXT session: {help}"
    );
}
