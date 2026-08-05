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

pub(crate) fn create_project_files(cwd: &Path) -> io::Result<String> {
    create_project_files_with(cwd, create_private_file)
}

fn create_project_files_with(
    cwd: &Path,
    mut write_file: impl FnMut(&Path, &str) -> io::Result<()>,
) -> io::Result<String> {
    let directory = cwd.join(".saya");
    let config = directory.join("config.toml");
    let connections = directory.join("connections.toml");
    if config.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            ".saya/config.toml already exists",
        ));
    }
    if connections.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            ".saya/connections.toml already exists",
        ));
    }

    let created_directory = !directory.exists();
    if created_directory {
        create_private_directory(&directory)?;
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
            let _ = fs::remove_dir(&directory);
        }
        return Err(error);
    }
    Ok("Created .saya/config.toml and .saya/connections.toml".into())
}

pub(crate) fn error_message(error: &io::Error) -> String {
    if error.kind() == io::ErrorKind::AlreadyExists {
        error.to_string()
    } else {
        "config init failed: could not create project templates".into()
    }
}

fn create_private_directory(path: &Path) -> io::Result<()> {
    fs::create_dir(path)?;
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
