use super::*;

#[cfg(not(windows))]
pub(super) struct Captured {
    pub(super) spawn_error: Option<String>,
    pub(super) timed_out: bool,
    pub(super) code: Option<i32>,
    pub(super) stdout: String,
    pub(super) stderr: String,
}

#[cfg(not(windows))]
impl Captured {
    pub(super) fn exited_ok(&self) -> bool {
        !self.timed_out && self.spawn_error.is_none() && self.code == Some(0)
    }

    pub(super) fn summary(&self) -> String {
        if let Some(e) = &self.spawn_error {
            return format!("spawn failed: {e}");
        }
        let exit = if self.timed_out {
            "TIMEOUT".to_owned()
        } else {
            format!(
                "exit {}",
                self.code.map_or("?".to_owned(), |c| c.to_string())
            )
        };
        let mut out = format!("exit: {exit}");
        if !self.stdout.is_empty() {
            let _ = write!(out, "\nstdout:\n{}", indent(&self.stdout));
        }
        if !self.stderr.is_empty() {
            let _ = write!(out, "\nstderr:\n{}", indent(&self.stderr));
        }
        out
    }
}

#[cfg(not(windows))]
pub(super) fn run_bounded(cmd: &mut Command) -> Captured {
    let blank = Captured {
        spawn_error: None,
        timed_out: false,
        code: None,
        stdout: String::new(),
        stderr: String::new(),
    };
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("LC_ALL", "C")
        .env("LANG", "C");
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Captured {
                spawn_error: Some(e.to_string()),
                ..blank
            };
        }
    };
    let start = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {
                if start.elapsed() > COMMAND_TIMEOUT {
                    let _ = child.kill();
                    timed_out = true;
                    break child.wait().ok();
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Captured {
                    spawn_error: Some(e.to_string()),
                    ..blank
                };
            }
        }
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    Captured {
        spawn_error: None,
        timed_out,
        code: status.and_then(|s| s.code()),
        stdout: cap(&stdout),
        stderr: cap(&stderr),
    }
}

#[cfg(not(windows))]
pub(super) fn cap(text: &str) -> String {
    if text.len() <= DETAIL_CAP {
        return text.to_owned();
    }
    let mut end = DETAIL_CAP;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n… (truncated at {DETAIL_CAP} bytes)", &text[..end])
}

#[cfg(not(windows))]
pub(super) fn uname_line() -> String {
    let out = run_bounded(Command::new("uname").arg("-a"));
    if out.exited_ok() {
        out.stdout.trim().to_owned()
    } else {
        format!(
            "{} {} (uname unavailable: {})",
            std::env::consts::OS,
            std::env::consts::ARCH,
            out.summary()
        )
    }
}
