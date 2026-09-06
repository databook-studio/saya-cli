use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::Path,
};

const CONFIG_TEMPLATE: &str = r#"default_profile = "analytics"

[ai]
provider = "ollama"
model = "qwen2.5-coder:14b"
base_url = "http://localhost:11434"
allow_data_sharing = false
# Sampling temperature (0.0–2.0). Lower = more concise/deterministic and
# usually faster; higher = more varied. Defaults to 0.1 when omitted.
temperature = 0.1

[run]
read_only = true
max_rows = 1000

[ui]
# TUI colour palette: dark, light, or auto (auto honours COLORFGBG and
# falls back to dark). The --theme flag overrides this for one invocation.
theme = "auto"
"#;

const CONNECTIONS_TEMPLATE: &str = r#"[profiles.analytics]
type = "postgresql"
host = "localhost"
port = 5432
database = "warehouse"
user = "saya_readonly"
password = { env = "SAYA_ANALYTICS_PASSWORD" }
sslmode = "require"
"#;

/// Writes starter templates to the user config directory (the trusted layer),
/// so a fresh `config init` followed by any command does not warn. This is the
/// default — see S18. Returns a message naming where the files went.
pub(crate) fn create_user_files(user_dir: &Path) -> io::Result<String> {
    create_files_with(
        user_dir,
        /* create_parents */ true,
        create_private_file,
        |dir| {
            format!(
                "Created config.toml and connections.toml in {}",
                dir.display()
            )
        },
    )
}

/// Writes starter templates to this project's `.saya/` (the untrusted layer).
/// Reachable via `config init --project` for team-shared, non-secret settings
/// checked into a repository. A command run afterward warns until
/// `--trust-project-config` is passed — that is the trust boundary doing its
/// job, not a bug.
pub(crate) fn create_project_files(cwd: &Path) -> io::Result<String> {
    create_files_with(
        &cwd.join(".saya"),
        /* create_parents */ false,
        create_private_file,
        |_| "Created .saya/config.toml and .saya/connections.toml".into(),
    )
}

fn create_files_with(
    directory: &Path,
    create_parents: bool,
    mut write_file: impl FnMut(&Path, &str) -> io::Result<()>,
    message: impl Fn(&Path) -> String,
) -> io::Result<String> {
    let config = directory.join("config.toml");
    let connections = directory.join("connections.toml");
    if config.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "config.toml already exists",
        ));
    }
    if connections.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "connections.toml already exists",
        ));
    }

    let created_directory = !directory.exists();
    if created_directory {
        create_private_directory(directory, create_parents)?;
    }
    let mut created = Vec::new();
    let result = (|| {
        write_file(&config, CONFIG_TEMPLATE)?;
        created.push(config.clone());
        write_file(&connections, CONNECTIONS_TEMPLATE)?;
        created.push(connections.clone());
        Ok::<(), io::Error>(())
    })();
    if let Err(error) = result {
        for path in created {
            let _ = fs::remove_file(path);
        }
        if created_directory {
            let _ = fs::remove_dir(directory);
        }
        return Err(error);
    }
    Ok(message(directory))
}

pub(crate) fn error_message(error: &io::Error) -> String {
    if error.kind() == io::ErrorKind::AlreadyExists {
        error.to_string()
    } else {
        "config init failed: could not create starter templates".into()
    }
}

fn create_private_directory(path: &Path, create_parents: bool) -> io::Result<()> {
    if create_parents {
        fs::create_dir_all(path)?;
    } else {
        fs::create_dir(path)?;
    }
    if let Err(error) = set_private_directory(path) {
        let _ = fs::remove_dir(path);
        return Err(error);
    }
    Ok(())
}

fn create_private_file(path: &Path, contents: &str) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    if let Err(error) = file
        .write_all(contents.as_bytes())
        .and_then(|_| file.sync_all())
    {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error);
    }
    Ok(())
}

#[cfg(unix)]
fn set_private_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn set_private_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
#[path = "init_tests.rs"]
mod tests;
