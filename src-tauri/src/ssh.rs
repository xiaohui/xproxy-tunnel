use crate::{
    model::Config,
    supervisor::{Driver, Session},
};
use anyhow::{ensure, Context};
use async_trait::async_trait;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    io::AsyncReadExt,
    process::{Child, Command},
    task::JoinHandle,
};

pub fn ssh_args(c: &Config, known: &Path) -> Vec<OsString> {
    let keypath = known
        .to_string_lossy()
        .replace('\\', "/")
        .replace('"', "\\\"");
    let mut args = vec![
        "-F".into(),
        "none".into(),
        "-N".into(),
        "-T".into(),
        "-D".into(),
        format!("127.0.0.1:{}", c.socks_port).into(),
        "-i".into(),
        c.key_path.clone().into(),
        "-p".into(),
        c.ssh_port.to_string().into(),
        "-l".into(),
        c.username.clone().into(),
    ];
    for opt in [
        "BatchMode=yes".to_owned(),
        "IdentitiesOnly=yes".into(),
        "StrictHostKeyChecking=accept-new".into(),
        "ExitOnForwardFailure=yes".into(),
        "ConnectTimeout=10".into(),
        "ServerAliveInterval=10".into(),
        "ServerAliveCountMax=2".into(),
        "ControlMaster=no".into(),
        "ControlPath=none".into(),
        "GlobalKnownHostsFile=none".into(),
        format!("UserKnownHostsFile=\"{keypath}\""),
        "LogLevel=ERROR".into(),
    ] {
        args.push("-o".into());
        args.push(opt.into());
    }
    args.push(c.host.clone().into());
    args
}
fn ssh_program() -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into()))
            .join("System32/OpenSSH/ssh.exe")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/usr/bin/ssh")
    }
}
pub struct SshDriver {
    pub dir: PathBuf,
}
struct SshSession {
    child: Child,
    errors: Arc<Mutex<String>>,
    reader: JoinHandle<()>,
    client: reqwest::Client,
    url: String,
}
impl Drop for SshSession {
    fn drop(&mut self) {
        // EOF not SIGKILL tells the guardian to reap OpenSSH, including on cancelled handshakes.
        self.child.stdin.take();
        self.reader.abort();
    }
}
#[async_trait]
impl Driver for SshDriver {
    async fn open(&self, c: &Config) -> anyhow::Result<Box<dyn Session>> {
        let executable = ssh_program();
        ensure!(
            executable.is_file(),
            "缺少 OpenSSH 客户端；Windows 请启用可选功能 OpenSSH Client"
        );
        let mut cmd = Command::new(std::env::current_exe()?);
        #[cfg(unix)]
        {
            cmd.process_group(0);
        }
        cmd.arg("--ssh-guardian")
            .arg(&executable)
            .args(ssh_args(c, &self.dir.join("known_hosts")))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            cmd.creation_flags(0x08000000);
        }
        // Do not kill-on-drop the guardian. Closing stdin lets it clean up its child.
        let mut child = cmd.spawn().context("无法启动 SSH 连接守护进程")?;
        let mut stderr = child.stderr.take().context("SSH stderr unavailable")?;
        let errors = Arc::new(Mutex::new(String::new()));
        let captured = errors.clone();
        let reader = tokio::spawn(async move {
            let mut buf = [0; 1024];
            while let Ok(n) = stderr.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                let mut s = captured.lock().expect("stderr mutex");
                s.push_str(&String::from_utf8_lossy(&buf[..n]));
                if s.len() > 8192 {
                    let mut at = s.len() - 8192;
                    while !s.is_char_boundary(at) {
                        at += 1
                    }
                    s.drain(..at);
                }
            }
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .proxy(reqwest::Proxy::all(format!(
                "socks5h://127.0.0.1:{}",
                c.socks_port
            ))?)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(8))
            .build()?;
        let mut session = SshSession {
            child,
            errors,
            reader,
            client,
            url: c.probe_url.clone(),
        };
        let deadline = Instant::now() + Duration::from_secs(18);
        loop {
            if let Some(e) = session.exited()? {
                anyhow::bail!("{e}")
            }
            match session.health().await {
                Ok(_) => {
                    if let Some(e) = session.exited()? {
                        anyhow::bail!("{e}")
                    }
                    return Ok(Box::new(session));
                }
                Err(e) if Instant::now() >= deadline => {
                    anyhow::bail!("无法通过 SSH 访问探测地址：{e:#}")
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(250)).await,
            }
        }
    }
}
#[async_trait]
impl Session for SshSession {
    async fn health(&mut self) -> anyhow::Result<u64> {
        let now = Instant::now();
        let response = self.client.get(&self.url).send().await?;
        ensure!(
            response.status().is_success(),
            "探测地址返回 HTTP {}",
            response.status()
        );
        Ok(now.elapsed().as_millis() as u64)
    }
    fn exited(&mut self) -> anyhow::Result<Option<String>> {
        Ok(self.child.try_wait()?.map(|code| {
            let detail = self.errors.lock().expect("stderr mutex").trim().to_owned();
            if detail.is_empty() {
                format!("SSH 已退出（{code}）")
            } else {
                detail
            }
        }))
    }
    async fn close(&mut self) {
        self.child.stdin.take();
        if tokio::time::timeout(Duration::from_secs(4), self.child.wait())
            .await
            .is_err()
        {
            #[cfg(unix)]
            {
                if let Some(pid) = self.child.id() {
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGKILL);
                    }
                }
            }
            #[cfg(windows)]
            {
                if let Some(pid) = self.child.id() {
                    let program = PathBuf::from(
                        std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into()),
                    )
                    .join("System32/taskkill.exe");
                    let _ = Command::new(program)
                        .args(["/PID", &pid.to_string(), "/T", "/F"])
                        .creation_flags(0x08000000)
                        .status()
                        .await;
                }
            }
            let _ = self.child.kill().await;
        }
        self.reader.abort();
    }
}
/// The pipe is a parent-lifetime lease: it closes even when the GUI crashes or is force-quit.
pub async fn guardian(program: OsString, args: Vec<OsString>) -> i32 {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        command.creation_flags(0x08000000);
    }
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("无法启动 OpenSSH：{e}");
            return 1;
        }
    };
    let mut input = tokio::io::stdin();
    let mut b = [0];
    tokio::select! {
        s=child.wait()=>s.ok().and_then(|s|s.code()).unwrap_or(1),
        _=input.read(&mut b)=>{let _=child.kill().await;let _=child.wait().await;0},
    }
}
