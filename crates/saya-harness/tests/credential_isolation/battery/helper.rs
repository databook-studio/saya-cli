use super::*;

/// The allowlisted runner program: the battery's compiled helper.
pub(super) const PROGRAM: &str = "saya-probe";
/// The tool's configured default timeout in the battery.
pub(super) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

/// The runner-side reference source for the orchestrator's credential —
/// what a production resolver would read from the user's environment.
pub(super) const ORCHESTRATOR_SOURCE: &str = "SAYA_E2E_ORCHESTRATOR_SOURCE";
/// The planted orchestrator credential: distinctive, searchable, and
/// deliberately carrying **no** redaction marker (`password=`, `api_key=`,
/// a credential header, URL userinfo) — a bare value must be scrubbed even
/// when the only thing naming it is the env variable it was injected
/// under.
pub(super) const ORCHESTRATOR_VALUE: &str = "planted-orchestrator-7Qk2mVn9xR4e2e";
/// The reviewer role's source and planted value (role separation only).
pub(super) const REVIEWER_SOURCE: &str = "SAYA_E2E_REVIEWER_SOURCE";
pub(super) const REVIEWER_VALUE: &str = "planted-reviewer-Zx4WpHj3Le2e2";

// -- the compiled helper -------------------------------------------------

/// The battery's helper: a digest dumper (FNV-1a 64 of one env value —
/// proves the child's environment held exactly the planted value without
/// the value ever being printed), a name-only presence prober, and an
/// exfiltration mode that echoes the recognised secret shapes plus the
/// credential under its real env name. One binary, compiled with the
/// toolchain running the tests.
pub(super) const HELPER_SOURCE: &str = r#"
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("");
    match mode {
        "digest" => {
            let var = args.get(2).cloned().unwrap_or_default();
            match std::env::var(&var) {
                Ok(value) => println!("DIGEST {} {:016x}", var, fnv1a(value.as_bytes())),
                Err(_) => println!("DIGEST {} UNSET", var),
            }
        }
        "which" => {
            for var in args.iter().skip(2) {
                match std::env::var(var) {
                    Ok(_) => println!("SET {}", var),
                    Err(_) => println!("UNSET {}", var),
                }
            }
        }
        "leak" => {
            let var = args.get(2).cloned().unwrap_or_default();
            println!("password=hunter2-marker-shape");
            println!("Authorization: Bearer sk-live-shape-marker");
            println!("https://user:hunter2-url-marker@db.example.com/x");
            println!("{}={}", var, std::env::var(&var).unwrap_or_default());
        }
        _ => {
            eprintln!("unknown mode");
            std::process::exit(2);
        }
    }
}
"#;

pub(super) fn helper() -> &'static PathBuf {
    static HELPER: OnceLock<PathBuf> = OnceLock::new();
    HELPER.get_or_init(|| {
        let base = std::env::var("CARGO_TARGET_TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir());
        let dir = base.join(format!("saya-credential-helper-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("helper directory must be creatable");
        let source = dir.join("saya-credential-helper.rs");
        let binary = dir.join("saya-credential-helper");
        fs::write(&source, HELPER_SOURCE).expect("helper source must be written");
        let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
        let status = Command::new(rustc)
            .arg("--edition=2021")
            .arg("-o")
            .arg(&binary)
            .arg(&source)
            .status()
            .expect("rustc must be runnable — it is the toolchain running this test");
        assert!(
            status.success(),
            "the battery helper must compile: {status}"
        );
        binary
    })
}

/// The digest the helper computes of an env value; test and helper must
/// agree, so the child's own report of its environment is the assertion.
pub(super) fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
