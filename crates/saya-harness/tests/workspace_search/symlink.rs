//! The walk's symlink discipline: an outside-pointing symlink is neither
//! visited by name nor descended into, so it can neither match nor be read.

use super::Sandbox;

#[cfg(unix)]
#[test]
fn the_walk_never_visits_or_descends_a_symlink() {
    use std::fs;
    use std::os::unix::fs::symlink;

    let sandbox = Sandbox::new("walk-symlink");
    fs::write(sandbox.outer.join("outside.md"), b"outside needle")
        .expect("outside plant must succeed");
    fs::create_dir_all(sandbox.outer.join("outside_dir")).expect("outside dir must create");
    fs::write(
        sandbox.outer.join("outside_dir").join("inside.md"),
        b"inside needle",
    )
    .expect("outside plant must succeed");
    symlink("../outside.md", sandbox.ws.root().join("decoy.md"))
        .expect("decoy symlink must create");
    symlink("../outside_dir", sandbox.ws.root().join("linkdir"))
        .expect("linkdir symlink must create");
    sandbox
        .ws
        .write("real/inner.md", b"real needle")
        .expect("contained write must succeed");
    let paths = sandbox
        .ws
        .glob("**/*.md", 100, 100)
        .expect("a contained glob must succeed")
        .into_iter()
        .map(|matched| matched.path)
        .collect::<Vec<_>>();
    assert_eq!(paths, vec!["real/inner.md"]);
    let outcome = sandbox
        .ws
        .grep("needle", false, 100, 100, 4096, 2_000)
        .expect("a contained grep must succeed");
    assert_eq!(outcome.files_scanned, 1);
    assert_eq!(outcome.matches.len(), 1);
    assert_eq!(outcome.matches[0].path, "real/inner.md");
}
