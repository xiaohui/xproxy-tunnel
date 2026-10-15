//! Developer acceptance runner. Uses a supplied test profile, never applies system proxies.
use std::{path::PathBuf, sync::Arc, time::Duration};
use xproxy::{
    model::{Config, Phase},
    supervisor::{Controller, ProxyOps, Timing},
};
struct NoSystem;
impl ProxyOps for NoSystem {
    fn apply(&self, _: u16) -> anyhow::Result<()> {
        anyhow::bail!("test runner refuses system mutation")
    }
    fn restore(&self) -> anyhow::Result<()> {
        Ok(())
    }
    fn active(&self) -> anyhow::Result<bool> {
        Ok(false)
    }
}
fn main() -> anyhow::Result<()> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() == Some(std::ffi::OsStr::new("--ssh-guardian")) {
        let Some(program) = args.next() else {
            std::process::exit(2)
        };
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        let code = runtime.block_on(xproxy::ssh::guardian(program, args.collect()));
        std::process::exit(code);
    }
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    anyhow::ensure!(
        args.len() == 2,
        "usage: tunnel_check <config.json> <isolated-data-dir>"
    );
    let mut config: Config = serde_json::from_slice(&std::fs::read(&args[0])?)?;
    config.system_proxy = false;
    let dir = PathBuf::from(&args[1]);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let c = Controller::new_with(
            dir.clone(),
            Arc::new(xproxy::ssh::SshDriver { dir }),
            Arc::new(NoSystem),
            Timing::default(),
        )?;
        c.save(config).await?;
        c.set_enabled(true).await?;
        let result = tokio::time::timeout(Duration::from_secs(35), async {
            loop {
                let s = c.status().await;
                println!("{}", serde_json::to_string(&s)?);
                if matches!(s.phase, Phase::Connected) {
                    return Ok::<_, anyhow::Error>(());
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        })
        .await;
        c.set_enabled(false).await?;
        result??;
        anyhow::ensure!(!c.status().await.desired_on, "failed to stop");
        Ok(())
    })
}
