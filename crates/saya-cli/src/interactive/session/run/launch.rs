/// Spawns the nested `saya run <tail…>` child, streams its output through
/// unmangled, and waits for it. The child's exit code is its own: it already
/// said why the run ended (its settle message names completed, paused, the
/// resume hint, or the failure), and the parent adds nothing to the stream —
/// any echo here would be exactly the re-tagging the module doc refuses.
/// `seed_forwarded` is the session grants a requested seed forwards: appended
/// as one more `--allow` group, the child's parser the authority on every
/// token.
pub(crate) fn spawn_run_child(
    runtime: &RuntimeConfig,
    format: RenderFormat,
    state: &SessionState,
    seed_forwarded: &[String],
    tail: &str,
) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let mut command = Command::new(exe);
    command
        .arg("--non-interactive")
        .arg("--format")
        .arg(format_flag(format))
        .arg("run")
        .args(
            child_argv(tail)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?,
        );
    if !seed_forwarded.is_empty() {
        command.arg("--allow").args(seed_forwarded);
    }
    // Forward the config sources the session actually loaded, so the child
    // resolves the same profiles, provider, and secrets — a run must not
    // silently run against a different configuration than the session's.
    if let Some(config) = &runtime.config_path {
        command.arg("--config").arg(config);
    }
    if let Some(connections) = &runtime.connections_path {
        command.arg("--connections").arg(connections);
    }
    if let Some(profile) = state.profile.as_deref() {
        command.arg("--profile").arg(profile);
    }
    // Forward the session's approval mode so the child resolves what the
    // session resolved — except bypass: a run's approval is its `--allow`
    // scopes (`commands/run/start.rs`), so a bypass session's blanket
    // per-call consent never reaches the child, which takes its own default
    // (read-only). The session's bypass never claimed to reach the child;
    // the run states its own scopes.
    if let Some(mode) = forwarded_approval_mode(state.approval_mode.as_str()) {
        command.arg("--approval-mode").arg(mode);
    }
    // The child never reads stdin (a headless run prompts for nothing), and
    // it must never consume the session's remaining input lines: null stdin.
    command
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    command.status().map(|_| ())
}

/// Shapes the child's `run` argument vector from the slash tail:
/// a subcommand tail passes through as tokenized argv (the child's parser reads
/// its id), and a goal tail rejoins its leading words into the single
/// positional the CLI declares — a goal is one string, then the flags.
fn child_argv(tail: &str) -> Result<Vec<String>, String> {
    let tokens = tokenize_tail(tail)?;
    if matches!(tokens.first(), Some(first) if SUBCOMMAND_WORDS.contains(&first.as_str())) {
        return Ok(tokens);
    }
    let boundary = tokens
        .iter()
        .position(|token| token.starts_with("--"))
        .unwrap_or(tokens.len());
    let mut argv = vec![tokens[..boundary].join(" ")];
    argv.extend(tokens[boundary..].iter().cloned());
    Ok(argv)
}

/// Splits a slash tail into argv words without invoking a shell. Quotes group
/// whitespace and backslashes escape the next character; both are removed.
/// Shell expansion and metacharacter handling do not exist here.
pub(crate) fn tokenize_tail(tail: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut started = false;

    for character in tail.chars() {
        if escaped {
            token.push(character);
            escaped = false;
            started = true;
            continue;
        }
        match quote {
            Some(delimiter) if character == delimiter => quote = None,
            Some(_) => token.push(character),
            None if character == '\\' => {
                escaped = true;
                started = true;
            }
            None if matches!(character, '\'' | '"') => {
                quote = Some(character);
                started = true;
            }
            None if character.is_whitespace() => {
                if started {
                    tokens.push(std::mem::take(&mut token));
                    started = false;
                }
            }
            None => {
                token.push(character);
                started = true;
            }
        }
    }
    if escaped {
        return Err("unmatched escape in /run tail".to_string());
    }
    if quote.is_some() {
        return Err("unmatched quote in /run tail".to_string());
    }
    if started {
        tokens.push(token);
    }
    Ok(tokens)
}

/// The approval mode forwarded to a nested `saya run` child: the session's
/// mode verbatim — except bypass. A run's approval is its `--allow` scopes;
/// a bypass session's blanket consent never reaches the child, which states
/// its own scopes or takes the run default (`app.rs`). `None` forwards
/// nothing.
fn forwarded_approval_mode(mode: &str) -> Option<&str> {
    (mode != "bypass").then_some(mode)
}

/// The format flag the child inherits, so a piped session's `/run` speaks the
/// same wire the session itself speaks.
fn format_flag(format: RenderFormat) -> &'static str {
    match format {
        RenderFormat::Text => "text",
        RenderFormat::Json => "json",
        RenderFormat::Ndjson => "ndjson",
    }
}
