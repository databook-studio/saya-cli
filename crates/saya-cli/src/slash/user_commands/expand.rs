/// Expands a command's body into the prompt the agent receives. Every
/// `$ARGUMENTS` is replaced by the text typed after the command; a body with
/// no placeholder gets that text appended, so `/name foo` never silently
/// drops what was typed. The result is bounded: an over-cap expansion is
/// refused, never truncated.
fn expand(command: &UserCommand, args: &str) -> Result<String, String> {
    let expanded = if command.body.contains(ARGUMENTS_PLACEHOLDER) {
        command.body.replace(ARGUMENTS_PLACEHOLDER, args)
    } else if args.is_empty() {
        command.body.clone()
    } else {
        format!("{}\n\n{args}", command.body)
    };
    let prompt = expanded.trim().to_string();
    if prompt.is_empty() {
        return Err(format!("/{} expands to an empty prompt", command.name));
    }
    if prompt.len() > MAX_EXPANDED_PROMPT_BYTES {
        return Err(format!(
            "/{} expands to {} bytes, over the {MAX_EXPANDED_PROMPT_BYTES}-byte limit",
            command.name,
            prompt.len()
        ));
    }
    Ok(prompt)
}
