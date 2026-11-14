/// The `/help` listing's user-command section, empty when none loaded — a
/// session with no `commands/` directory prints exactly today's listing.
pub(crate) fn user_commands_section(commands: &UserCommands) -> String {
    let mut lines = String::new();
    for command in commands.ready() {
        lines.push_str("\n  /");
        lines.push_str(&command.name);
        if let Some(description) = &command.description {
            lines.push_str(" — ");
            lines.push_str(description);
        }
    }
    if lines.is_empty() {
        return lines;
    }
    format!("\n\nUser commands (their body is sent as your message):{lines}")
}

/// The `/help <name>` text for one loaded user command: what it does and where
/// it came from, so a project command is never mistaken for a built-in.
pub(crate) fn user_command_help(command: &UserCommand) -> String {
    let mut out = format!("/{}", command.name);
    if let Some(description) = &command.description {
        out.push_str(" — ");
        out.push_str(description);
    }
    let origin = match command.source {
        CommandSource::Builtin => {
            "A built-in command; a user or repository command with the same name \
             replaces it."
                .to_owned()
        }
        source => {
            let replaces = if is_builtin_name(&command.name) {
                " It replaces the built-in command of the same name."
            } else {
                ""
            };
            format!(
                "A {} command from {}.{replaces}",
                source.label(),
                command.path.display()
            )
        }
    };
    out.push_str(&format!(
        "\n{origin} Its body is sent to the agent as your message; \
         $ARGUMENTS is replaced by the text you type after the command."
    ));
    out
}
