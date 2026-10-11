use super::{DEADLINE, Provider};
use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    process::Command,
    sync::mpsc,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const OUTPUT_LIMIT: usize = 256 * 1024;
const INPUT_LIMIT: usize = 32 * 1024;
const IO_WAIT: Duration = Duration::from_millis(100);

struct Root(PathBuf);
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub struct Session {
    root: Root,
    child: Option<Box<dyn Child + Send + Sync>>,
    writer: Option<Box<dyn Write + Send>>,
    output: mpsc::Receiver<Vec<u8>>,
    reader: Option<JoinHandle<()>>,
    raw: Vec<u8>,
}

impl Session {
    pub fn new(label: &str, config: &str, provider: &Provider) -> Self {
        let root = std::env::temp_dir().join(format!(
            "saya-coding-controls-pty-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("home")).unwrap();
        fs::create_dir_all(root.join("repo")).unwrap();
        fs::write(root.join("home/config.toml"), config).unwrap();
        let git = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(root.join("repo"))
            .status()
            .expect("installed git initializes the private PTY repository");
        assert!(
            git.success(),
            "private PTY repository initialization succeeds"
        );
        fs::write(root.join("repo/baseline.txt"), b"tracked baseline\n").unwrap();
        for args in [
            ["config", "user.name", "Saya PTY Fixture"],
            ["config", "user.email", "pty@example.invalid"],
            ["add", "baseline.txt"],
            ["commit", "--quiet", "-m", "baseline"],
        ] {
            let status = Command::new("git")
                .args(args)
                .current_dir(root.join("repo"))
                .status()
                .expect("git baseline command starts");
            assert!(status.success(), "git baseline command succeeds");
        }

        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: 30,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let config_path = root.join("home/config.toml");
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_saya"));
        command.args([
            "--config",
            config_path.to_str().unwrap(),
            "--workspace",
            root.join("repo").to_str().unwrap(),
            "--approval-mode",
            "bypass",
            "--verbose",
        ]);
        command.cwd(root.join("repo"));
        command.env("HOME", root.join("home"));
        command.env("SAYA_CONFIG_HOME", root.join("home"));
        command.env("SAYA_STATE_DB", root.join("home/state.sqlite3"));
        command.env_remove("SAYA_API_KEY");
        command.env_remove("SAYA_AI_API_KEY");
        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let writer = pair.master.take_writer().unwrap();
        let reader = pair.master.try_clone_reader().unwrap();
        let (tx, output) = mpsc::channel();
        let reader = thread::spawn(move || drain(reader, tx));
        let _ = provider;
        Self {
            root: Root(root),
            child: Some(child),
            writer: Some(writer),
            output,
            reader: Some(reader),
            raw: Vec::new(),
        }
    }

    pub fn send(&mut self, bytes: &[u8]) {
        assert!(bytes.len() <= INPUT_LIMIT, "one PTY input is bounded");
        let writer = self.writer.as_mut().expect("PTY writer remains live");
        writer.write_all(bytes).unwrap();
        writer.flush().unwrap();
    }

    pub fn wait_for(&mut self, needle: &str, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if String::from_utf8_lossy(&self.raw).contains(needle) {
                return;
            }
            self.pump();
        }
        panic!("PTY did not show {needle:?}; tail:\n{}", self.tail());
    }

    pub fn finish(&mut self) -> Vec<u8> {
        self.send(b"\x03\x03");
        let deadline = Instant::now() + DEADLINE;
        while Instant::now() < deadline {
            if self
                .child
                .as_mut()
                .and_then(|child| child.try_wait().ok())
                .flatten()
                .is_some()
            {
                break;
            }
            self.pump();
        }
        self.cleanup(Duration::from_secs(2));
        self.raw.clone()
    }

    pub fn records(&self, job: &str) -> String {
        let root = self.root.0.join("home/coding").join(job);
        let mut output = Vec::new();
        let entries = fs::read_dir(root).expect("owned coding records directory exists");
        for entry in entries.take(128) {
            let entry = entry.unwrap();
            if !entry.file_type().unwrap().is_file() {
                continue;
            }
            let bytes = fs::read(entry.path()).unwrap();
            if bytes.len() <= 1024 * 1024 {
                output.extend_from_slice(&bytes);
            }
            assert!(output.len() <= 4 * 1024 * 1024, "records are bounded");
        }
        String::from_utf8_lossy(&output).into_owned()
    }

    pub fn state_db(&self) -> PathBuf {
        self.root.0.join("home/state.sqlite3")
    }

    fn pump(&mut self) {
        if let Ok(chunk) = self.output.recv_timeout(IO_WAIT) {
            self.raw.extend_from_slice(&chunk);
            assert!(self.raw.len() <= OUTPUT_LIMIT, "PTY transcript is bounded");
        }
    }

    fn tail(&self) -> String {
        let start = self.raw.len().saturating_sub(4096);
        String::from_utf8_lossy(&self.raw[start..]).into_owned()
    }

    fn cleanup(&mut self, drain_for: Duration) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let deadline = Instant::now() + drain_for;
            while Instant::now() < deadline {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) => thread::sleep(Duration::from_millis(10)),
                    Err(_) => break,
                }
            }
        }
        self.child = None;
        self.writer.take();
        let deadline = Instant::now() + drain_for;
        let mut reader_finished = false;
        while Instant::now() < deadline {
            match self.output.recv_timeout(IO_WAIT) {
                Ok(chunk) => {
                    self.raw.extend_from_slice(&chunk);
                    if self.raw.len() >= OUTPUT_LIMIT {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    reader_finished = true;
                    break;
                }
            }
        }
        if reader_finished {
            if let Some(reader) = self.reader.take() {
                let _ = reader.join();
            }
        } else {
            self.reader.take();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.cleanup(Duration::from_millis(100));
    }
}

fn drain(mut reader: Box<dyn Read + Send>, tx: mpsc::Sender<Vec<u8>>) {
    let mut total = 0;
    let mut buffer = [0_u8; 4096];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                let remaining = OUTPUT_LIMIT.saturating_sub(total);
                let end = count.min(remaining);
                if end == 0 || tx.send(buffer[..end].to_vec()).is_err() {
                    break;
                }
                total += end;
                if total >= OUTPUT_LIMIT {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}
