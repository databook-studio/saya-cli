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
    writer: Box<dyn Write + Send>,
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
            writer,
            output,
            reader: Some(reader),
            raw: Vec::new(),
        }
    }

    pub fn send(&mut self, bytes: &[u8]) {
        assert!(bytes.len() <= INPUT_LIMIT, "one PTY input is bounded");
        self.writer.write_all(bytes).unwrap();
        self.writer.flush().unwrap();
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

    pub fn finish(mut self) -> Vec<u8> {
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
        self.cleanup();
        self.raw
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

    fn cleanup(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.child = None;
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.cleanup();
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
