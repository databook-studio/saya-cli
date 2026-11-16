/// Loads the user-level and project-level command directories. The order is
/// fixed and matters: the user's own directory first, so a project file can
/// never displace a command the user wrote.
pub(crate) fn load(user_dir: &Path, project_dir: Option<&Path>) -> UserCommands {
    let mut commands = UserCommands::default();
    commands.load_dir(CommandSource::User, user_dir);
    if let Some(dir) = project_dir {
        commands.load_dir(CommandSource::Project, dir);
    }
    commands
}

impl UserCommands {
    fn load_dir(&mut self, source: CommandSource, dir: &Path) {
        let mut paths = match command_paths(dir) {
            Ok(paths) => paths,
            Err(notice) => {
                self.notice(notice);
                return;
            }
        };
        // Bound the work one directory can cause: a repository may hold any
        // number of `.md` files, and the first `MAX_COMMAND_FILES` in path
        // order are the ones considered.
        if paths.len() > MAX_COMMAND_FILES {
            paths.truncate(MAX_COMMAND_FILES);
            self.notice(format!(
                "{}: more than {MAX_COMMAND_FILES} command files; only the first \
                 {MAX_COMMAND_FILES} in name order are considered",
                dir.display()
            ));
        }
        for path in paths {
            self.load_file(source, &path);
        }
    }

    fn load_file(&mut self, source: CommandSource, path: &Path) {
        let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) else {
            self.notice(format!(
                "refused {}: the file name is not UTF-8",
                path.display()
            ));
            return;
        };
        if !is_safe_name(name) {
            self.notice(format!(
                "refused {}: a command name uses lowercase letters, digits, and `-`, \
                 at most {MAX_COMMAND_NAME_CHARS} characters",
                path.display()
            ));
            return;
        }
        if registry::KNOWN_COMMANDS.contains(&name) {
            self.notice(format!(
                "refused {}: /{name} is a built-in command, which a command file \
                 cannot replace",
                path.display()
            ));
            return;
        }
        if let Some(entry) = self.entries.get(name) {
            if entry.is_builtin() {
                // A shipped default is only a default: the user's own file,
                // or the repository's, replaces it by name.
                self.entries.remove(name);
                self.notice(format!(
                    "{}: /{name} replaces the built-in command",
                    path.display()
                ));
            } else {
                let origin = self.origin(name).unwrap_or_default();
                self.notice(format!(
                    "refused {}: /{name} is already defined by {origin}",
                    path.display()
                ));
                return;
            }
        }
        if self.entries.len() >= MAX_COMMAND_FILES {
            self.notice(format!(
                "refused {}: at most {MAX_COMMAND_FILES} commands load",
                path.display()
            ));
            return;
        }
        match read_command(path) {
            Ok(text) => self.load_text(source, name, path, &text),
            Err(reason) => self.refuse(name, path, reason),
        }
    }

    fn load_text(&mut self, source: CommandSource, name: &str, path: &Path, text: &str) {
        let (keys, body) = match split_front_matter(text) {
            Ok(parts) => parts,
            Err(reason) => return self.refuse(name, path, reason),
        };
        let mut description = None;
        let mut unknown: Vec<String> = Vec::new();
        for (key, value) in keys {
            match key.as_str() {
                // The one key this loader reads. Everything else is ignored
                // with a diagnostic: a command file's front matter is text,
                // never a setting, and saying so is how a writer learns their
                // key did nothing.
                "description" => description = (!value.is_empty()).then_some(value),
                other => {
                    if !unknown.iter().any(|seen| seen == other) {
                        unknown.push(other.to_string());
                    }
                }
            }
        }
        if !unknown.is_empty() {
            self.notice(format!(
                "{}: ignoring unknown front matter key(s): {}",
                path.display(),
                unknown.join(", ")
            ));
        }
        let body = body.trim().to_string();
        if body.is_empty() {
            return self.refuse(name, path, "it has no prompt body".into());
        }
        self.entries.insert(
            name.to_string(),
            Entry::Ready(UserCommand {
                name: name.to_string(),
                description,
                body,
                source,
                path: path.to_path_buf(),
            }),
        );
    }

    /// Refuses a file that claimed a name: the name stays reserved so a later
    /// file cannot take it, and typing it says why instead of "unknown
    /// command".
    fn refuse(&mut self, name: &str, path: &Path, reason: String) {
        self.notice(format!("refused {}: {reason}", path.display()));
        self.entries.insert(
            name.to_string(),
            Entry::Refused {
                path: path.to_path_buf(),
                reason,
            },
        );
    }

    /// The path of the file that already holds this name, ready or refused.
    fn origin(&self, name: &str) -> Option<String> {
        match self.entries.get(name) {
            Some(Entry::Ready(command)) => Some(command.path.display().to_string()),
            Some(Entry::Refused { path, .. }) => Some(path.display().to_string()),
            None => None,
        }
    }
}

impl Entry {
    /// Whether this entry is a shipped default, the one kind a file may
    /// replace.
    fn is_builtin(&self) -> bool {
        matches!(
            self,
            Entry::Ready(UserCommand {
                source: CommandSource::Builtin,
                ..
            })
        )
    }
}

impl UserCommands {

    /// Records one diagnostic, bounded: a hostile directory cannot flood a
    /// session's startup with them.
    fn notice(&mut self, message: String) {
        if self.notices.len() >= MAX_COMMAND_NOTICES {
            if !self.notices_suppressed {
                self.notices_suppressed = true;
                self.notices.push(format!(
                    "further command-file diagnostics were suppressed \
                     (at most {MAX_COMMAND_NOTICES} are reported)"
                ));
            }
            return;
        }
        self.notices.push(message);
    }
}

/// The `.md` files directly inside `dir`, sorted by path. A missing directory
/// is not an error: no `commands/` directory is the normal case.
fn command_paths(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("could not read {}: {error}", dir.display())),
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
        .collect();
    paths.sort();
    Ok(paths)
}

/// Reads one command file under the byte cap. The file must be a regular file —
/// a symlink, a FIFO, or a device is refused, never read, so a project
/// `commands/x.md` cannot point the prompt at a file outside the tree — and
/// it is never truncated: a file over the cap is refused whole.
fn read_command(path: &Path) -> Result<String, String> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // No-follow refuses a symlink at the open itself; nonblocking refuses
        // a FIFO instead of trapping the session on it.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    let file = options.open(path).map_err(|error| read_error(&error))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("it could not be read: {error}"))?;
    if !metadata.is_file() {
        return Err("it is not a regular file".into());
    }
    if metadata.len() > MAX_COMMAND_FILE_BYTES as u64 {
        return Err(too_large(metadata.len()));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize + 1);
    file.take(MAX_COMMAND_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("it could not be read: {error}"))?;
    if bytes.len() > MAX_COMMAND_FILE_BYTES {
        return Err(too_large(bytes.len() as u64));
    }
    String::from_utf8(bytes).map_err(|_| "it is not UTF-8".into())
}

/// The refusal for an open that failed. On Unix a no-follow open of a symlink
/// fails with `ELOOP`, which is the refusal, not an I/O error.
#[cfg(unix)]
fn read_error(error: &io::Error) -> String {
    if error.raw_os_error() == Some(libc::ELOOP) {
        return "it is a symlink, which a command file cannot be".into();
    }
    format!("it could not be read: {error}")
}

#[cfg(not(unix))]
fn read_error(error: &io::Error) -> String {
    format!("it could not be read: {error}")
}

fn too_large(bytes: u64) -> String {
    format!("it is {bytes} bytes, over the {MAX_COMMAND_FILE_BYTES}-byte limit")
}
