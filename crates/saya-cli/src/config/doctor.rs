use super::runtime::RuntimeConfig;

pub(crate) fn summary(runtime: &RuntimeConfig) -> String {
    format!(
        "config: {}\nconnections: {}\nprofiles: {}\nselected profile: {}",
        path(&runtime.config_path),
        path(&runtime.connections_path),
        runtime.connections.profiles.len(),
        runtime.resolved.profile_name.as_deref().unwrap_or("none")
    )
}

fn path(value: &Option<std::path::PathBuf>) -> String {
    value
        .as_deref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "not found".into())
}
