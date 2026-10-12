//! Spawn and teardown plumbing for the private PTY session fixture.

use super::{OUTPUT_LIMIT, Root, Session};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::{fs, io::Read, process::Command, sync::mpsc, thread};

impl Session {
    pub fn new(label: &str, config: &str, provider: &super::super::Provider) -> Self {
        let root = std::env::temp_dir().join(format!(
            "saya-coding-controls-pty-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("home/saya")).unwrap();
        fs::create_dir_all(root.join("repo")).unwrap();
        fs::write(root.join("home/saya/config.toml"), config).unwrap();
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
            ["config", "user.name", "Saya PTY Fixture"].as_slice(),
            ["config", "user.email", "pty@example.invalid"].as_slice(),
            ["add", "baseline.txt"].as_slice(),
            ["commit", "--quiet", "-m", "baseline"].as_slice(),
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
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_saya"));
        command.args([
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
            child_reaped: false,
            reader_finished: false,
        }
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
