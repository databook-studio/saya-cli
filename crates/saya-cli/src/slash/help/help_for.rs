/// Returns command-specific help for a topic, or general help text if `topic`
/// is `None`. The session's loaded user commands join the listing and answer
/// for their own names; a built-in always wins a name it holds, because a
/// command file can never take one.
pub(crate) fn help_for(topic: Option<&str>, commands: &UserCommands) -> String {
    match topic {
        Some(name) => {
            let clean = name.trim_start_matches('/');
            if let Some(command) = commands.get(clean) {
                return user_command_help(command);
            }
            match command_help(clean) {
                Some(help) => help.to_string(),
                None => format!("No help for /{clean}. Type /help for the full list."),
            }
        }
        None => format!("{}{}", help_text(), user_commands_section(commands)),
    }
}
