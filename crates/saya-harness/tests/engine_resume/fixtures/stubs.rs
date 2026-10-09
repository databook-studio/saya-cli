/// The stub collaborators every resume under test carries. The executors
/// live in the per-step toolsets (one `NoTools` behind a shared `Arc`), so
/// no executor field is needed here.
#[derive(Default)]
struct Stubs {
    provider: AnswerProvider,
    approval: AllowApproval,
}

fn bounds() -> ManifestBounds {
    ManifestBounds {
        max_files: 8,
        max_file_bytes: 64 * 1024,
    }
}

fn workspace(root: &std::path::Path) -> Workspace {
    let dir = root.join("workspace");
    fs::create_dir_all(&dir).unwrap();
    Workspace::open(&dir).unwrap()
}

fn request() -> EpisodeRequest {
    EpisodeRequest {
        model: "mock-model".into(),
        profile_names: Vec::new(),
        memory_allows_candidate_writes: false,
    }
}
