use super::*;

// -- the compiled stand-in for the `saya` binary ------------------------

/// The battery's one helper program, compiled with the same toolchain
/// running the tests and staged as `saya` in the program directory: it
/// accepts the nested argv shape and reports what it could actually
/// open — its cwd, its environment (pinned paths raw, every other value
/// hashed so no captured lane carries a secret), each file it was
/// pointed at, the user layer its own resolution implies, its cwd's
/// `.saya/` discovery, and one probe path (the question slot doubles as
/// a probe path). A file it manages to open is echoed byte for byte, so
/// a leak reaches the capture and the test sees it.
const HELPER_SOURCE: &str = r#"
use std::{env, fs};
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
fn open_report(label: &str, path: &str) {
    match fs::read_to_string(path) {
        Ok(text) => {
            println!("open:{label}:{path}:ok");
            print!("{text}");
            if !text.is_empty() && !text.ends_with('\n') {
                println!();
            }
        }
        Err(error) => println!("open:{label}:{path}:denied({error})"),
    }
}
fn main() {
    let args: Vec<String> = env::args().collect();
    if args.get(1).map(String::as_str) != Some("ask") {
        eprintln!("the nested argv shape is `ask ...`; got {:?}", args.get(1));
        std::process::exit(2);
    }
    let (mut config, mut connections, mut question) =
        (String::new(), String::new(), String::new());
    let mut index = 2;
    while index < args.len() {
        match args[index].as_str() {
            "--config" => {
                config = args.get(index + 1).cloned().unwrap_or_default();
                index += 2;
            }
            "--connections" => {
                connections = args.get(index + 1).cloned().unwrap_or_default();
                index += 2;
            }
            "--profile" | "--approval-mode" | "--format" => index += 2,
            "--non-interactive" | "--trust-project-config" => index += 1,
            other => {
                question = other.to_owned();
                index += 1;
            }
        }
    }
    for (index, arg) in args.iter().enumerate() {
        let hex: String = arg.bytes().map(|b| format!("{b:02x}")).collect();
        println!("arg{index}={hex}");
    }
    println!("cwd={}", env::current_dir().unwrap().display());
    for name in ["SAYA_CONFIG_HOME", "SAYA_STATE_DB", "SAYA_SESSION_DIR"] {
        println!("env-pin:{name}={}", env::var(name).unwrap_or_else(|_| "<unset>".into()));
    }
    let mut vars: Vec<(String, String)> = env::vars().collect();
    vars.sort();
    for (name, value) in vars {
        if matches!(
            name.as_str(),
            "SAYA_CONFIG_HOME" | "SAYA_STATE_DB" | "SAYA_SESSION_DIR"
        ) {
            continue;
        }
        println!("env-hashed:{name}:{}:{:016x}", value.len(), fnv1a(value.as_bytes()));
    }
    open_report("config", &config);
    open_report("connections", &connections);
    if let Ok(home) = env::var("SAYA_CONFIG_HOME") {
        open_report("user-config", &format!("{home}/saya/config.toml"));
        open_report("user-connections", &format!("{home}/saya/connections.toml"));
    }
    let discovered = env::current_dir().unwrap().join(".saya/connections.toml");
    println!(
        "discover:{}:{}",
        discovered.display(),
        if discovered.exists() { "found" } else { "missing" }
    );
    if !question.is_empty() {
        open_report("probe", &question);
    }
}
"#;

pub(super) fn leak(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("saya-nested-test-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("battery root must be creatable");
    fs::canonicalize(&path).expect("battery root must canonicalise")
}

pub(super) fn scope() -> RunnerScope {
    RunnerScope::new(vec![NESTED_PROGRAM.to_owned()]).expect("shaped")
}

pub(super) fn helper() -> &'static PathBuf {
    static HELPER: OnceLock<PathBuf> = OnceLock::new();
    HELPER.get_or_init(|| {
        let base = std::env::var("CARGO_TARGET_TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir());
        let dir = base.join(format!("saya-nested-helper-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("helper directory must be creatable");
        let source = dir.join("saya-nested-helper.rs");
        let binary = dir.join("saya-nested-helper");
        fs::write(&source, HELPER_SOURCE).expect("helper source must be written");
        let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
        let status = Command::new(rustc)
            .arg("--edition=2021")
            .arg("-o")
            .arg(&binary)
            .arg(&source)
            .status()
            .expect("rustc must be runnable — it is the toolchain running this test");
        assert!(status.success(), "the nested helper must compile: {status}");
        binary
    })
}
