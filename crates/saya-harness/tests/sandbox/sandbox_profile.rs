use super::*;

/// Test 6 — a half-rendered profile must not silently become a permissive
/// one: the generator refuses leftovers (the necessary guard), sandbox-exec
/// rejects an unquoted leftover at parse, and a quoted leftover is accepted
/// by sandbox-exec and matches nothing — the measured fail-open mode that
/// makes the generator-side guard mandatory.
#[cfg(target_os = "macos")]
#[test]
fn a_half_rendered_profile_is_refused_by_the_generator_and_by_sandbox_exec() {
    use saya_harness::runner::sandbox::macos;

    // Injection refusal at construction: a root whose text could alter the
    // profile language is refused, never escaped (escaping is unmeasured).
    let paren = TempRoot::new("paren");
    let inside_paren = paren.canonical().join("sub(close-paren)");
    fs::create_dir_all(&inside_paren).expect("the paren-named dir must be creatable");
    assert!(
        matches!(
            RunSandbox::new([inside_paren], Vec::<(String, u16)>::new()),
            Err(SandboxError::RootNotSafeForProfile { .. })
        ),
        "a root containing profile-language characters must be refused at construction"
    );

    // The generator emits no leftover placeholders, and says what it does
    // not cover: the net_allow host component is enforced in-process only —
    // in the header when there are no entries, and on every rule when there
    // are.
    let root = TempRoot::new("gen");
    let sb = RunSandbox::new([root.canonical()], Vec::<(String, u16)>::new())
        .expect("policy constructs");
    let profile = macos::seatbelt_profile(&sb, &[PathBuf::from("/bin")])
        .expect("a valid policy generates a profile");
    assert!(
        !profile.contains('{'),
        "the generated profile must carry no placeholder: {profile}"
    );
    assert!(
        profile.contains("enforced in-process by the M3-1 fetch policy"),
        "the profile must say what it does not cover (net_allow hosts): {profile}"
    );
    let sb_with_net = RunSandbox::new([root.canonical()], [("127.0.0.1".to_owned(), 8443)])
        .expect("policy with loopback entry constructs");
    let profile_with_net = macos::seatbelt_profile(&sb_with_net, &[PathBuf::from("/bin")])
        .expect("a valid policy generates a profile");
    assert!(
        profile_with_net.contains("host enforced in-process only"),
        "each net rule must say the host is enforced in-process only: {profile_with_net}"
    );

    // An unquoted leftover is rejected at parse (measured: exit 65,
    // "unbound variable").
    let unquoted = format!("{profile}\n(allow process-exec (subpath {{SAYA_LEFTOVER}}))\n");
    let out = bounded(
        Command::new("/usr/bin/sandbox-exec")
            .arg("-p")
            .arg(&unquoted)
            .arg("/bin/echo")
            .arg("x")
            .current_dir(root.canonical()),
    );
    assert_eq!(
        out.status.code(),
        Some(65),
        "an unquoted leftover must be rejected at parse: {out:?}"
    );

    // A quoted leftover is silently accepted and matches nothing (measured
    // on this host: exit 0) — the fail-open mode. The generator's scan is
    // what catches it, so the scan must reject this exact text.
    let quoted = format!("{profile}\n(allow file-read* (subpath \"{{SAYA_LEFT}}\"))\n");
    let out = bounded(
        Command::new("/usr/bin/sandbox-exec")
            .arg("-p")
            .arg(&quoted)
            .arg("/bin/echo")
            .arg("x")
            .current_dir(root.canonical()),
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "the measured fail-open mode: sandbox-exec accepts a quoted leftover and it \
         matches nothing — which is why the generator refuses placeholders itself"
    );
    assert!(
        macos::has_leftover_placeholder(&quoted),
        "the generator-side scan must refuse the text sandbox-exec silently accepts"
    );
}
