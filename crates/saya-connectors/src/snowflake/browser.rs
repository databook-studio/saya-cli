pub(crate) fn open(url: &str) -> Result<(), ()> {
    #[cfg(target_os = "macos")]
    let command = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let command = std::process::Command::new("rundll32.exe")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let command = std::process::Command::new("xdg-open").arg(url).spawn();
    command.map(|_| ()).map_err(|_| ())
}
