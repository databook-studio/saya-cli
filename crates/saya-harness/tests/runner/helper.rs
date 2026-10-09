    // -- the compiled helper ------------------------------------------------

    /// The battery's one helper program, compiled with the same toolchain
    /// running the tests: an argv dumper (hex per element), an env dumper,
    /// a cwd printer, a stdout flood, a sleeper, a forking daemonizer, and
    /// an exit-code setter. One binary, staged into the program directory.
    const HELPER_SOURCE: &str = r#"
use std::io::Write;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("");
    match mode {
        "argv" => {
            println!("argc={}", args.len());
            for (index, arg) in args.iter().enumerate() {
                let hex: String = arg.bytes().map(|b| format!("{b:02x}")).collect();
                println!("arg{index}={hex}");
            }
        }
        "env" => {
            let mut vars: Vec<(String, String)> = std::env::vars().collect();
            vars.sort();
            for (name, value) in vars {
                println!("{name}={value}");
            }
        }
        "cwd" => println!("{}", std::env::current_dir().unwrap().display()),
        "sleep" => {
            let seconds: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
            std::thread::sleep(Duration::from_secs(seconds));
        }
        "flood" => {
            let bytes: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            let chunk = vec![b'x'; 4096];
            let mut out = std::io::stdout();
            let mut left = bytes;
            while left >= chunk.len() {
                out.write_all(&chunk).unwrap();
                left -= chunk.len();
            }
            out.write_all(&chunk[..left]).unwrap();
            println!("\nflooded");
        }
        "secret" => {
            println!("password=hunter2");
            println!("Authorization: Bearer sk-live-999");
            println!("https://user:hunter2@db.example.com/x");
        }
        "fork-daemon" => unsafe {
            if fork() == 0 {
                sleep(60);
                std::process::exit(0);
            }
            sleep(60);
        },
        "fork-orphan" => unsafe {
            if fork() == 0 {
                sleep(60);
                std::process::exit(0);
            }
        },
        "exit" => {
            let code: i32 = args.get(2).and_then(|c| c.parse().ok()).unwrap_or(0);
            std::process::exit(code);
        }
        _ => {
            eprintln!("unknown mode");
            std::process::exit(2);
        }
    }
}

#[cfg(unix)]
extern "C" {
    fn fork() -> i32;
    fn sleep(seconds: u32);
}

use std::time::Duration;
"#;

    fn helper() -> &'static PathBuf {
        static HELPER: OnceLock<PathBuf> = OnceLock::new();
        HELPER.get_or_init(|| {
            let base = std::env::var("CARGO_TARGET_TMPDIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| std::env::temp_dir());
            let dir = base.join(format!("saya-runner-helper-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("helper directory must be creatable");
            let source = dir.join("saya-runner-helper.rs");
            let binary = dir.join("saya-runner-helper");
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
