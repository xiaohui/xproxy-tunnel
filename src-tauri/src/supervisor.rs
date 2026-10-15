use crate::{
    model::{retry_seconds, Config, Phase, Status},
    storage,
    transport::{serve_bridge, Gate},
};
use anyhow::{ensure, Context};
use async_trait::async_trait;
use fs2::FileExt;
use std::{
    fs::{File, OpenOptions},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    net::TcpListener,
    sync::{Mutex, Notify},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

#[async_trait]
pub trait Session: Send {
    async fn health(&mut self) -> anyhow::Result<u64>;
    fn exited(&mut self) -> anyhow::Result<Option<String>>;
    async fn close(&mut self);
}
#[async_trait]
pub trait Driver: Send + Sync {
    async fn open(&self, config: &Config) -> anyhow::Result<Box<dyn Session>>;
}
pub trait ProxyOps: Send + Sync {
    fn apply(&self, port: u16) -> anyhow::Result<()>;
    fn restore(&self) -> anyhow::Result<()>;
    fn active(&self) -> anyhow::Result<bool>;
    fn pending_recovery(&self) -> anyhow::Result<bool> {
        self.active()
    }
    fn ensure_applied(&self, port: u16) -> anyhow::Result<()> {
        if self.pending_recovery()? {
            ensure!(self.active()?, "系统代理设置已被外部修改，未覆盖外部设置");
            Ok(())
        } else {
            self.apply(port)
        }
    }
    fn retry_application(&self, port: u16) -> anyhow::Result<()> {
        ensure!(
            !self.pending_recovery()?,
            "系统代理设置有待恢复，请先关闭代理再重试，避免覆盖外部修改"
        );
        self.apply(port)
    }
}
/// Every journal read/check/write transaction shares this lock with command-side retries.
struct SerialProxy {
    inner: Arc<dyn ProxyOps>,
    operation: std::sync::Mutex<()>,
}
impl SerialProxy {
    fn call<T>(&self, f: impl FnOnce(&dyn ProxyOps) -> anyhow::Result<T>) -> anyhow::Result<T> {
        let _guard = self
            .operation
            .lock()
            .map_err(|_| anyhow::anyhow!("系统代理操作锁已损坏"))?;
        f(self.inner.as_ref())
    }
}
impl ProxyOps for SerialProxy {
    fn apply(&self, port: u16) -> anyhow::Result<()> {
        self.call(|p| p.apply(port))
    }
    fn restore(&self) -> anyhow::Result<()> {
        self.call(|p| p.restore())
    }
    fn active(&self) -> anyhow::Result<bool> {
        self.call(|p| p.active())
    }
    fn pending_recovery(&self) -> anyhow::Result<bool> {
        self.call(|p| p.pending_recovery())
    }
    fn ensure_applied(&self, port: u16) -> anyhow::Result<()> {
        self.call(|p| p.ensure_applied(port))
    }
    fn retry_application(&self, port: u16) -> anyhow::Result<()> {
        self.call(|p| p.retry_application(port))
    }
}
impl ProxyOps for crate::system::SystemProxy {
    fn apply(&self, port: u16) -> anyhow::Result<()> {
        Self::apply(self, port)
    }
    fn restore(&self) -> anyhow::Result<()> {
        Self::restore(self)
    }
    fn active(&self) -> anyhow::Result<bool> {
        Self::active(self)
    }
    fn pending_recovery(&self) -> anyhow::Result<bool> {
        Ok(self.dir.join("system-proxy.json").exists())
    }
}
#[derive(Clone)]
pub struct Timing {
    pub health: Duration,
    pub tick: Duration,
    pub backoff_unit: Duration,
}
impl Default for Timing {
    fn default() -> Self {
        Self {
            health: Duration::from_secs(10),
            tick: Duration::from_millis(200),
            backoff_unit: Duration::from_secs(1),
        }
    }
}
struct Running {
    stop: CancellationToken,
    task: JoinHandle<()>,
    retry: Arc<Notify>,
    gate: Arc<Gate>,
}
pub struct Controller {
    dir: PathBuf,
    config: Mutex<Config>,
    status: Arc<Mutex<Status>>,
    running: Mutex<Option<Running>>,
    driver: Arc<dyn Driver>,
    proxy: Arc<dyn ProxyOps>,
    timing: Timing,
    _lock: File,
}
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
impl Controller {
    pub fn new_with(
        dir: PathBuf,
        driver: Arc<dyn Driver>,
        proxy: Arc<dyn ProxyOps>,
        timing: Timing,
    ) -> anyhow::Result<Self> {
        storage::private_dir(&dir)?;
        let mut opts = OpenOptions::new();
        opts.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let lock = opts.open(dir.join("instance.lock"))?;
        lock.try_lock_exclusive()
            .context("XProxy 已在运行，请使用已经打开的窗口")?;
        let proxy: Arc<dyn ProxyOps> = Arc::new(SerialProxy {
            inner: proxy,
            operation: std::sync::Mutex::new(()),
        });
        let config = storage::read_json(&dir.join("config.json"))?.unwrap_or_default();
        let recovery = proxy
            .restore()
            .err()
            .map(|e| format!("恢复原系统代理失败：{e:#}"));
        let status = Status {
            proxy_error: recovery,
            ..Status::default()
        };
        Ok(Self {
            dir,
            config: Mutex::new(config),
            status: Arc::new(Mutex::new(status)),
            running: Mutex::new(None),
            driver,
            proxy,
            timing,
            _lock: lock,
        })
    }
    pub async fn config(&self) -> Config {
        self.config.lock().await.clone()
    }
    pub async fn status(&self) -> Status {
        self.status.lock().await.clone()
    }
    pub async fn pending_recovery(&self) -> bool {
        let p = self.proxy.clone();
        !matches!(
            tokio::task::spawn_blocking(move || p.pending_recovery()).await,
            Ok(Ok(false))
        )
    }
    pub async fn save(&self, config: Config) -> anyhow::Result<Config> {
        let running = self.running.lock().await;
        ensure!(running.is_none(), "修改配置前请先关闭代理");
        config.validate()?;
        storage::write_json(&self.dir.join("config.json"), &config)?;
        *self.config.lock().await = config.clone();
        Ok(config)
    }
    pub async fn set_enabled(&self, enabled: bool) -> anyhow::Result<()> {
        let mut running = self.running.lock().await;
        if enabled {
            if running.is_some() {
                return Ok(());
            }
            let config = self.config().await;
            config.validate()?;
            // Preserve ownership: no second supervisor can mutate the recovery journal.
            let proxy = self.proxy.clone();
            tokio::task::spawn_blocking(move || proxy.restore()).await??;
            let socks = (std::net::Ipv4Addr::LOCALHOST, config.socks_port).into();
            let reservation = TcpListener::bind(socks)
                .await
                .context("本地 SOCKS5 端口已被占用")?;
            drop(reservation);
            let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, config.http_port))
                .await
                .context("本地 HTTP 代理端口已被占用")?;
            let stop = CancellationToken::new();
            let retry = Arc::new(Notify::new());
            let gate = Gate::new();
            *self.status.lock().await = Status {
                desired_on: true,
                phase: Phase::Connecting,
                ..Status::default()
            };
            let bridge = tokio::spawn(serve_bridge(listener, socks, gate.clone(), stop.clone()));
            let task = tokio::spawn(run(
                config,
                self.driver.clone(),
                self.proxy.clone(),
                self.status.clone(),
                gate.clone(),
                stop.clone(),
                retry.clone(),
                self.timing.clone(),
                bridge,
            ));
            *running = Some(Running {
                stop,
                task,
                retry,
                gate,
            });
            Ok(())
        } else {
            if let Some(runtime) = running.take() {
                {
                    let mut s = self.status.lock().await;
                    s.desired_on = false;
                    s.phase = Phase::Stopping;
                    s.retry_at_ms = None;
                    s.latency_ms = None;
                    s.connect_ms = None;
                    s.connected_at_ms = None;
                }
                runtime.gate.close();
                runtime.stop.cancel();
                if let Err(e) = runtime.task.await {
                    let p = self.proxy.clone();
                    let recovery = tokio::task::spawn_blocking(move || p.restore()).await;
                    let mut s = self.status.lock().await;
                    s.last_error = Some(format!("代理任务退出失败：{e}"));
                    s.proxy_error = match recovery {
                        Ok(Ok(())) => None,
                        Ok(Err(e)) => Some(e.to_string()),
                        Err(e) => Some(e.to_string()),
                    };
                }
            } else {
                let p = self.proxy.clone();
                let result = tokio::task::spawn_blocking(move || p.restore()).await?;
                self.status.lock().await.proxy_error = result.err().map(|e| e.to_string());
            }
            let mut s = self.status.lock().await;
            s.desired_on = false;
            s.phase = Phase::Off;
            s.retry_at_ms = None;
            if let Some(e) = &s.proxy_error {
                anyhow::bail!("{e}")
            }
            Ok(())
        }
    }
    pub async fn retry_now(&self) -> anyhow::Result<()> {
        let runtime = self.running.lock().await;
        ensure!(runtime.is_some(), "请先开启代理");
        let current = self.status().await;
        if matches!(current.phase, Phase::Connected)
            && current.proxy_error.is_some()
            && !current.system_proxy_active
        {
            let config = self.config().await;
            ensure!(config.system_proxy, "未选择系统代理模式");
            let p = self.proxy.clone();
            let result =
                tokio::task::spawn_blocking(move || p.retry_application(config.http_port)).await?;
            let mut s = self.status.lock().await;
            s.system_proxy_active = result.is_ok();
            s.proxy_error = result
                .as_ref()
                .err()
                .map(|e| format!("系统代理未启用：{e:#}"));
            return result;
        }
        runtime.as_ref().unwrap().retry.notify_one();
        Ok(())
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        if let Some(r) = self.running.get_mut().take() {
            r.gate.close();
            r.stop.cancel();
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run(
    config: Config,
    driver: Arc<dyn Driver>,
    proxy: Arc<dyn ProxyOps>,
    status: Arc<Mutex<Status>>,
    gate: Arc<Gate>,
    stop: CancellationToken,
    retry: Arc<Notify>,
    timing: Timing,
    mut bridge: JoinHandle<anyhow::Result<()>>,
) {
    let mut failures = 0u32;
    let mut attempts = 0u32;
    let mut proxy_applied = false;
    loop {
        if stop.is_cancelled() {
            break;
        }
        attempts = attempts.saturating_add(1);
        {
            let mut s = status.lock().await;
            s.attempt = attempts;
            s.phase = if attempts == 1 {
                Phase::Connecting
            } else {
                Phase::Reconnecting
            };
            s.retry_at_ms = None;
        }
        let start = Instant::now();
        let repair = if bridge.is_finished() {
            match TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, config.http_port)).await {
                Ok(listener) => {
                    bridge = tokio::spawn(serve_bridge(
                        listener,
                        (std::net::Ipv4Addr::LOCALHOST, config.socks_port).into(),
                        gate.clone(),
                        stop.clone(),
                    ));
                    Ok(())
                }
                Err(e) => Err(anyhow::Error::from(e).context("本地 HTTP 代理恢复失败")),
            }
        } else {
            Ok(())
        };
        let open = match repair {
            Err(e) => Err(e),
            Ok(()) => tokio::select! {_=stop.cancelled()=>break,r=driver.open(&config)=>r},
        };
        let failure = match open {
            Err(e) => format!("SSH 连接失败：{e:#}"),
            Ok(mut session) => {
                let initial = tokio::select! {_=stop.cancelled()=>{session.close().await;break},r=session.health()=>r};
                match initial {
                    Err(e) => {
                        session.close().await;
                        format!("代理探测失败：{e:#}")
                    }
                    Ok(latency) => {
                        // Never publish connected merely because SSH spawned or localhost is listening.
                        gate.ready();
                        let connect_ms = start.elapsed().as_millis() as u64;
                        if config.system_proxy && !proxy_applied {
                            let p = proxy.clone();
                            let result = tokio::task::spawn_blocking(move || {
                                p.ensure_applied(config.http_port)
                            })
                            .await;
                            let outcome = result.map_err(anyhow::Error::from).and_then(|r| r);
                            proxy_applied = outcome.is_ok();
                            let mut s = status.lock().await;
                            s.proxy_error = outcome.err().map(|e| format!("系统代理未启用：{e:#}"));
                            s.system_proxy_active = proxy_applied;
                        }
                        if stop.is_cancelled() {
                            gate.close();
                            session.close().await;
                            break;
                        }
                        {
                            let mut s = status.lock().await;
                            s.phase = Phase::Connected;
                            s.connect_ms = Some(connect_ms);
                            s.latency_ms = Some(latency);
                            s.connected_at_ms = Some(now_ms());
                            s.last_error = None;
                        }
                        failures = 0;
                        let mut next_probe = Instant::now() + timing.health;
                        let failure = loop {
                            tokio::select! {_=stop.cancelled()=>break None,_=tokio::time::sleep(timing.tick)=>{}}
                            if bridge.is_finished() {
                                break Some("本地 HTTP 代理中断，正在恢复".into());
                            }
                            match session.exited() {
                                Ok(Some(e)) => break Some(e),
                                Err(e) => break Some(e.to_string()),
                                Ok(None) => {}
                            }
                            if Instant::now() >= next_probe {
                                let health = tokio::select! {_=stop.cancelled()=>break None,r=session.health()=>r};
                                match health {
                                    Ok(ms) => status.lock().await.latency_ms = Some(ms),
                                    Err(e) => break Some(format!("代理连接中断：{e:#}")),
                                }
                                if config.system_proxy
                                    && (proxy_applied || status.lock().await.system_proxy_active)
                                {
                                    let p = proxy.clone();
                                    let active =
                                        tokio::task::spawn_blocking(move || p.active()).await;
                                    let mut s = status.lock().await;
                                    match active {
                                        Ok(Ok(true)) => s.system_proxy_active = true,
                                        Ok(Ok(false)) => {
                                            s.system_proxy_active = false;
                                            s.proxy_error = Some(
                                                "系统代理设置已被外部修改，未覆盖外部设置".into(),
                                            );
                                        }
                                        _ => {
                                            s.system_proxy_active = false;
                                            s.proxy_error = Some("无法确认系统代理状态".into());
                                        }
                                    }
                                }
                                next_probe = Instant::now() + timing.health;
                            }
                        };
                        gate.close();
                        session.close().await;
                        if let Some(e) = failure {
                            e
                        } else {
                            break;
                        }
                    }
                }
            }
        };
        gate.close();
        failures = failures.saturating_add(1);
        let wait = timing.backoff_unit * retry_seconds(failures) as u32;
        {
            let mut s = status.lock().await;
            s.failed(failure);
            s.retry_at_ms = Some(now_ms() + wait.as_millis() as u64);
        }
        tokio::select! {_=stop.cancelled()=>break,_=retry.notified()=>{},_=tokio::time::sleep(wait)=>{}}
    }
    gate.close();
    stop.cancel();
    let _ = bridge.await;
    let p = proxy.clone();
    let result = tokio::task::spawn_blocking(move || p.restore()).await;
    let mut s = status.lock().await;
    s.desired_on = false;
    s.phase = Phase::Off;
    s.latency_ms = None;
    s.connect_ms = None;
    s.connected_at_ms = None;
    s.retry_at_ms = None;
    s.system_proxy_active = false;
    s.proxy_error = match result {
        Ok(Ok(())) => None,
        Ok(Err(e)) => Some(format!("恢复原系统代理失败：{e:#}")),
        Err(e) => Some(e.to_string()),
    };
}
