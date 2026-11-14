pub struct Env {
    pub root: PathBuf,
    pub project: PathBuf,
    pub sessions: PathBuf,
}

pub fn env(name: &str) -> Env {
    let root = std::env::temp_dir().join(format!("saya-turn-coding-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let project = root.join("project");
    let sessions = root.join("sessions");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::create_dir_all(project.join(".git")).unwrap();
    std::fs::create_dir_all(&sessions).unwrap();
    Env {
        root,
        project,
        sessions,
    }
}
