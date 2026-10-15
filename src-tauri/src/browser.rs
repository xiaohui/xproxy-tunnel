use crate::storage;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
pub fn browser_args(profile: &Path, socks_port: u16) -> Vec<OsString> {
    vec![
        format!("--user-data-dir={}", profile.display()).into(),
        format!("--proxy-server=socks5://127.0.0.1:{socks_port}").into(),
        "--host-resolver-rules=MAP * ~NOTFOUND , EXCLUDE 127.0.0.1".into(),
        "--disable-quic".into(),
        "--force-webrtc-ip-handling-policy=disable_non_proxied_udp".into(),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "--new-window".into(),
        "https://example.com".into(),
    ]
}
fn candidates() -> Vec<(&'static str, PathBuf)> {
    #[cfg(target_os = "macos")]
    {
        let mut roots = vec![PathBuf::from("/Applications")];
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(home).join("Applications"));
        }
        roots
            .into_iter()
            .flat_map(|root| {
                [
                    (
                        "Google Chrome",
                        root.join("Google Chrome.app/Contents/MacOS/Google Chrome"),
                    ),
                    (
                        "Microsoft Edge",
                        root.join("Microsoft Edge.app/Contents/MacOS/Microsoft Edge"),
                    ),
                ]
            })
            .collect()
    }
    #[cfg(windows)]
    {
        ["LOCALAPPDATA", "PROGRAMFILES", "PROGRAMFILES(X86)"]
            .into_iter()
            .filter_map(std::env::var_os)
            .flat_map(|root| {
                let root = PathBuf::from(root);
                [
                    (
                        "Google Chrome",
                        root.join("Google/Chrome/Application/chrome.exe"),
                    ),
                    (
                        "Microsoft Edge",
                        root.join("Microsoft/Edge/Application/msedge.exe"),
                    ),
                ]
            })
            .collect()
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Vec::new()
    }
}
pub fn launch(dir: &Path, socks_port: u16) -> anyhow::Result<String> {
    let (name, program) = candidates()
        .into_iter()
        .find(|(_, path)| path.is_file())
        .ok_or_else(|| anyhow::anyhow!("未找到 Chrome 或 Edge，请先安装其中一个浏览器"))?;
    // Separate profile per port avoids a running profile reusing stale proxy arguments.
    let profile = dir.join(format!("browser-profile-{socks_port}"));
    storage::private_dir(&profile)?;
    let mut cmd = Command::new(program);
    cmd.args(browser_args(&profile, socks_port))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let mut child = cmd.spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(name.into())
}
