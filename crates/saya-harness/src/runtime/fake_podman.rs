use std::{env, fs, io::Write, path::PathBuf, process::Command, thread, time::Duration};

fn main() {
    let Ok(authfile) = env::var("REGISTRY_AUTH_FILE") else {
        std::process::exit(97);
    };
    let root = PathBuf::from(&authfile)
        .parent()
        .expect("authfile has parent")
        .to_path_buf();
    let arguments: Vec<String> = env::args().skip(1).collect();
    let _ = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("calls.log"))
        .and_then(|mut file| writeln!(file, "auth={authfile}; args={arguments:?}"));
    let Some(command) = arguments.first().map(String::as_str) else {
        std::process::exit(2);
    };
    match command {
        "__orphan" => thread::sleep(Duration::from_secs(30)),
        "container" if arguments.get(1).is_some_and(|arg| arg == "exists") => {
            if container_matches(&root, arguments.last().map(String::as_str).unwrap_or("")) {
                std::process::exit(0);
            }
            std::process::exit(plan(&root, "exists_exit", 1));
        }
        "container" if arguments.get(1).is_some_and(|arg| arg == "inspect") => {
            inspect_container(&root);
        }
        "container" if arguments.get(1).is_some_and(|arg| arg == "ps") => {
            list_containers(&root, &arguments);
        }
        "rm" => {
            let code = plan(&root, "rm_exit", 0);
            if code == 0
                && container_matches(&root, arguments.last().map(String::as_str).unwrap_or(""))
            {
                let _ = fs::remove_file(container_record(&root));
            } else if code == 0 {
                std::process::exit(1);
            }
            std::process::exit(code);
        }
        "pull" => std::process::exit(plan(&root, "pull_exit", 0)),
        "image" if arguments.get(1).is_some_and(|arg| arg == "inspect") => {
            println!(
                "{}",
                plan_text(
                    &root,
                    "digest",
                    "sha256:ba6b7d0ee561d9d49deb99e2b93c98632cc287b6cca1a6b18d66127054efa4dc"
                )
            );
            std::process::exit(plan(&root, "inspect_exit", 0));
        }
        "run" => run_probe(&root, &arguments),
        _ => std::process::exit(0),
    }
}

fn run_probe(root: &std::path::Path, args: &[String]) -> ! {
    let cidfile = args
        .windows(2)
        .find(|pair| pair[0] == "--cidfile")
        .map(|pair| PathBuf::from(&pair[1]));
    let ticket = args
        .windows(2)
        .find(|pair| pair[0] == "--label")
        .and_then(|pair| pair[1].split_once('=').map(|(_, value)| value.to_owned()))
        .unwrap_or_default();
    let name = args
        .windows(2)
        .find(|pair| pair[0] == "--name")
        .map(|pair| pair[1].clone())
        .unwrap_or_default();
    let id = "a".repeat(64);
    let no_create = plan(root, "run_no_create", 0) != 0;
    if plan(root, "run_orphan", 0) != 0 {
        let executable = env::current_exe().expect("fake Podman executable path");
        let _orphan = Command::new(executable)
            .arg("__orphan")
            .spawn()
            .expect("fake Podman orphan starts");
    }
    if !no_create {
        if let Some(cidfile) = cidfile {
            let _ = fs::write(cidfile, format!("{id}\n"));
        }
        let _ = fs::write(container_record(root), format!("{id}\n{ticket}\n{name}\n"));
    }
    let _ = fs::write(root.join("run-started"), b"started");
    thread::sleep(Duration::from_millis(plan(&root, "run_delay_ms", 0) as u64));
    let npm = args.iter().any(|arg| arg.ends_with("npm-cli.js"));
    println!(
        "{}",
        plan_text(
            root,
            if npm { "npm" } else { "node" },
            if npm { "11.19.0" } else { "v24.21.0" }
        )
    );
    let code = plan(root, "run_exit", 0);
    if code == 0 {
        let _ = fs::remove_file(container_record(root));
    }
    std::process::exit(code);
}

fn inspect_container(root: &std::path::Path) -> ! {
    let delay_ms = plan(root, "inspect_delay_ms", 0).max(0) as u64;
    if delay_ms > 0 {
        let _ = fs::write(root.join("inspect-started"), b"started");
        thread::sleep(Duration::from_millis(delay_ms));
        let _ = fs::write(root.join("inspect-completed"), b"completed");
    }
    let Ok(contents) = fs::read_to_string(container_record(root)) else {
        std::process::exit(125);
    };
    let inspect_exit = plan(root, "container_inspect_exit", 0);
    if inspect_exit != 0 {
        std::process::exit(inspect_exit);
    }
    let Some(locator) = env::args().last() else {
        std::process::exit(125);
    };
    let mut lines = contents.lines();
    let Some(id) = lines.next() else {
        std::process::exit(125);
    };
    let Some(ticket) = lines.next() else {
        std::process::exit(125);
    };
    let Some(name) = lines.next() else {
        std::process::exit(125);
    };
    if locator != id && locator != name {
        std::process::exit(1);
    }
    println!("{id}\t{name}\t{ticket}");
    std::process::exit(0);
}

fn list_containers(root: &std::path::Path, args: &[String]) -> ! {
    if let Ok(contents) = fs::read_to_string(container_record(root)) {
        let ticket_filter = args
            .windows(2)
            .find(|pair| pair[0] == "--filter")
            .and_then(|pair| pair[1].rsplit_once('=').map(|(_, ticket)| ticket));
        let mut lines = contents.lines();
        let id = lines.next();
        let ticket = lines.next();
        if ticket_filter.is_some_and(|filter| Some(filter) == ticket) {
            if let Some(id) = id {
                println!("{id}");
            }
        }
        std::process::exit(0);
    }
    std::process::exit(0);
}

fn container_matches(root: &std::path::Path, locator: &str) -> bool {
    if let Ok(contents) = fs::read_to_string(container_record(root)) {
        let mut lines = contents.lines();
        let id = lines.next();
        let _ticket = lines.next();
        let name = lines.next();
        id == Some(locator) || name == Some(locator)
    } else {
        false
    }
}

fn container_record(root: &std::path::Path) -> PathBuf {
    root.join("fake-owned-container-state")
}

fn plan(root: &std::path::Path, key: &str, default: i32) -> i32 {
    plan_text(root, key, &default.to_string())
        .parse()
        .unwrap_or(default)
}

fn plan_text(root: &std::path::Path, key: &str, default: &str) -> String {
    fs::read_to_string(root.join("fake-plan"))
        .ok()
        .and_then(|plan| {
            plan.lines()
                .find_map(|line| line.split_once('=').filter(|(name, _)| *name == key))
                .map(|(_, value)| value.to_owned())
        })
        .unwrap_or_else(|| default.to_owned())
}
