/// Returns command-specific help for a topic, or general help text if `topic` is `None`.
pub(crate) fn help_for(topic: Option<&str>) -> String {
    match topic {
        Some(name) => {
            let clean = name.trim_start_matches('/');
            match command_help(clean) {
                Some(help) => help.to_string(),
                None => format!("No help for /{clean}. Type /help for the full list."),
            }
        }
        None => help_text().to_string(),
    }
}
