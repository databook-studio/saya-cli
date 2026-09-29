use std::{
    fs,
    path::Path,
    path::PathBuf,
    process::{Command as ProcessCommand, Output},
};

pub(crate) fn test_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("saya-cli-{label}-{}", std::process::id()));
    if root.exists() {
        let _ = fs::remove_dir_all(&root);
    }
    fs::create_dir_all(&root).unwrap();
    root
}

pub(crate) fn saya_process(root: &Path, args: &[&str]) -> Output {
    ProcessCommand::new(env!("CARGO_BIN_EXE_saya"))
        .args(args)
        .current_dir(root)
        .env("SAYA_CONFIG_HOME", root.join("user-config"))
        .output()
        .unwrap()
}

pub(crate) fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

pub(crate) fn run_cli(global: &[&str], command: &[&str], state: &Path) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args(global)
        .args(command)
        .env("SAYA_STATE_DB", state)
        .output()
        .unwrap()
}

/// Reads one HTTP request from a mock's accepted connection to completion:
/// the headers, then exactly the `Content-Length` body bytes. The scripted
/// mocks must answer only after the whole request is drained — reqwest sends
/// the body after the headers, and a single short read leaves the socket with
/// unread bytes, so closing it resets the connection mid-request and the
/// client retries, consuming the scripted responses.
pub(crate) fn drain_request(stream: &mut std::net::TcpStream) {
    use std::io::Read;
    let (mut buffer, mut chunk) = (Vec::new(), [0_u8; 8192]);
    loop {
        let head = buffer.windows(4).position(|window| window == b"\r\n\r\n");
        if let Some(head) = head {
            let length = String::from_utf8_lossy(&buffer[..head])
                .lines()
                .find_map(|line| match line.split_once(':') {
                    Some((name, value)) if name.eq_ignore_ascii_case("content-length") => {
                        value.trim().parse::<usize>().ok()
                    }
                    _ => None,
                })
                .unwrap_or(0);
            eprintln!(
                "MOCK DEBUG: head={head} length={length} buffer={}",
                buffer.len()
            );
            if buffer.len() >= head + 4 + length {
                eprintln!(
                    "MOCK DEBUG: full request: {}",
                    String::from_utf8_lossy(&buffer)
                        .chars()
                        .take(1200)
                        .collect::<String>()
                );
                return;
            }
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
        }
    }
}
