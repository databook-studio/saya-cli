use super::*;

fn errno_text(errno: i32) -> String {
    std::io::Error::from_raw_os_error(errno).to_string()
}

fn read_record(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| format!("absent ({e})"))
}

fn landlock_abi() -> Result<u32, i32> {
    let r = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<libc::c_void>(),
            0usize,
            1u32,
        )
    };
    if r == -1 {
        Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0))
    } else {
        Ok(u32::try_from(r).unwrap_or(u32::MAX))
    }
}

unsafe fn child_fail(fd: libc::c_int) -> ! {
    unsafe {
        let errno = *libc::__errno_location();
        libc::write(fd, std::ptr::from_ref(&errno).cast::<libc::c_void>(), 4);
        libc::close(fd);
        libc::_exit(1);
    }
}

fn unprivileged_user_net_ns() -> Result<(), String> {
    unsafe {
        let mut fds: [libc::c_int; 2] = [0; 2];
        if libc::pipe(fds.as_mut_ptr()) != 0 {
            return Err(format!("pipe: {}", errno_text(*libc::__errno_location())));
        }
        match libc::fork() {
            -1 => Err(format!("fork: {}", errno_text(*libc::__errno_location()))),
            0 => {
                libc::close(fds[0]);
                if libc::unshare(libc::CLONE_NEWUSER) != 0 {
                    child_fail(fds[1]);
                }
                if libc::unshare(libc::CLONE_NEWNET) != 0 {
                    child_fail(fds[1]);
                }
                libc::close(fds[1]);
                libc::_exit(0);
            }
            pid => {
                libc::close(fds[1]);
                let mut buf = [0u8; 4];
                let mut got = 0usize;
                loop {
                    let n = libc::read(
                        fds[0],
                        buf.as_mut_ptr().add(got).cast::<libc::c_void>(),
                        4 - got,
                    );
                    if n < 0 {
                        if *libc::__errno_location() == libc::EINTR {
                            continue;
                        }
                        break;
                    }
                    if n == 0 {
                        break;
                    }
                    got += n as usize;
                    if got == 4 {
                        break;
                    }
                }
                libc::close(fds[0]);
                let mut status: libc::c_int = 0;
                loop {
                    if libc::waitpid(pid, &mut status, 0) == -1
                        && *libc::__errno_location() == libc::EINTR
                    {
                        continue;
                    }
                    break;
                }
                let errno = (got == 4).then(|| i32::from_ne_bytes(buf));
                let exited = (status & 0x7f) == 0;
                let code = ((status >> 8) & 0xff) as i32;
                let signal = status & 0x7f;
                if errno.is_none() && exited && code == 0 {
                    Ok(())
                } else if let Some(e) = errno {
                    Err(format!(
                        "unshare(CLONE_NEWUSER) + unshare(CLONE_NEWNET) failed: {}",
                        errno_text(e)
                    ))
                } else {
                    Err(format!(
                        "probe child did not exit cleanly (exited={exited}, \
                             code={code}, signal={signal})"
                    ))
                }
            }
        }
    }
}

pub(super) fn run() -> ProbeReport {
    let uname = uname_line();
    let mut checks = vec![Check::info("host_uname", uname.clone())];

    checks.push(match landlock_abi() {
        Ok(v) => Check::passed(
            "landlock_abi",
            true,
            format!(
                "landlock_create_ruleset(NULL, 0, LANDLOCK_CREATE_RULESET_VERSION) \
                     returned ABI version {v}"
            ),
        ),
        Err(e) => Check::failed(
            "landlock_abi",
            true,
            format!(
                "Landlock version query failed (errno {e}: {}): this kernel does not \
                     expose Landlock to unprivileged callers",
                errno_text(e)
            ),
        ),
    });

    let lsm = read_record("/sys/kernel/security/lsm");
    checks.push(Check::info(
        "lsm_list",
        format!(
            "landlock listed in /sys/kernel/security/lsm: {}\ncontent: {}",
            lsm.contains("landlock"),
            lsm.trim_end()
        ),
    ));
    checks.push(Check::info(
        "user_max_user_namespaces",
        read_record("/proc/sys/user/max_user_namespaces")
            .trim_end()
            .to_owned(),
    ));

    checks.push(match unprivileged_user_net_ns() {
        Ok(()) => Check::passed(
            "user_network_namespace_unprivileged",
            true,
            "a forked, unprivileged child ran unshare(CLONE_NEWUSER) then \
                 unshare(CLONE_NEWNET); both succeeded",
        ),
        Err(e) => Check::failed(
            "user_network_namespace_unprivileged",
            true,
            format!(
                "unprivileged user+network namespace creation failed: {e} — the \
                     netns egress mechanism is unavailable on this host"
            ),
        ),
    });

    checks.push(Check::failed(
        "landlock_enforcement",
        true,
        "not probed in this slice: proving a Landlock ruleset denies requires the \
             `landlock` dependency (which this spike must not add) and a Linux host to \
             run on. The checks above are availability measurements only; this probe \
             can never prove the runner on Linux, so Linux fails closed until the \
             implementation slice lands.",
    ));

    ProbeReport {
        platform: "linux",
        kernel: uname,
        checks,
    }
}
