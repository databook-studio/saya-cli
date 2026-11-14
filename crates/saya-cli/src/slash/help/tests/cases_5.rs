/// `/help start` must describe the shipped native coding surface. `--program`
/// left `saya coding start` with the runner and interpreter doors: the flags
/// that still parse are `--write`, `--database-profile` and `--approve`. And a
/// job's helpers work in the coordinator's one managed working checkout — they
/// are not given checkouts of their own. Help that still lists `--program`
/// sends a user to a flag that refuses, and "isolated managed checkouts"
/// describes a layout the job does not have.
#[test]
fn start_help_names_the_shipped_flags_and_the_shared_checkout() {
    let help = command_help("start").expect("start has per-command help");
    assert!(
        !help.contains("--program"),
        "the /start help must not list the removed --program flag: {help}"
    );
    for flag in ["--write", "--database-profile", "--approve"] {
        assert!(
            help.contains(flag),
            "the /start help names the flag that still parses, {flag}: {help}"
        );
    }
    assert!(
        !help.contains("isolated"),
        "helpers share the coordinator's checkout, so the /start help must not \
         claim isolation: {help}"
    );
    assert!(
        help.contains("one managed working checkout"),
        "the /start help states the shared checkout: {help}"
    );
}

/// `/help` lists the job surface the parser actually accepts: `/job`,
/// `/start`, `/follow-up`, and the bare controls — never the retired
/// `/coding`, which is refused with a pointer to these names.
#[test]
fn help_lists_the_job_surface_not_the_retired_coding_name() {
    let listing = help_text();
    for shown in ["/job ", "/start ", "/follow-up ", "/approve ", "/deny "] {
        assert!(
            listing.contains(shown),
            "the /help listing shows {shown}: {listing}"
        );
    }
    assert!(
        !listing.contains("/coding"),
        "the /help listing must not show the retired /coding spelling: {listing}"
    );
}

/// `/help reload` lists what a running session never changes. There is no
/// `[jobs.runner]` key: the runner capability does not exist, and the reload
/// classifier carries no runner key, so the list must not name a jobs runner:
/// a user reading it would
/// go looking for a setting that does not exist. The settings that DO wait
/// for a new session — data sharing, host commands and session denies, theme,
/// thinking display — stay named.
#[test]
fn reload_help_lists_no_retired_jobs_runner() {
    let help = command_help("reload").expect("reload has per-command help");
    assert!(
        !help.contains("jobs runner") && !help.contains("jobs.runner"),
        "the /reload help must not list the retired jobs runner: {help}"
    );
    for named in [
        "data sharing",
        "host commands",
        "session denies",
        "theme",
        "thinking display",
    ] {
        assert!(
            help.contains(named),
            "the /reload help still names {named} among what waits for a new \
             session: {help}"
        );
    }
}
