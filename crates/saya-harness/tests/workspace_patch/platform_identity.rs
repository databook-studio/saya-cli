#[cfg(windows)]
use super::*;

#[cfg(windows)]
#[test]
fn patching_an_existing_file_keeps_the_replacement_identity() {
    let sandbox = Sandbox::new("windows-replacement-identity");
    sandbox.ws.write("victim.txt", b"before").unwrap();

    sandbox
        .ws
        .patch_range("victim.txt", 0..6, 6, b"after")
        .expect("Windows replacement must retain the temporary file identity");

    assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), b"after");
}
