use async_trait::async_trait;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use xproxy::{
    model::{Config, Phase},
    supervisor::{Controller, Driver, ProxyOps, Session, Timing},
};

struct Fixture {
    opens: AtomicUsize,
    fail: AtomicBool,
    closes: Arc<AtomicUsize>,
}
struct Live {
    fail: bool,
    closes: Arc<AtomicUsize>,
}
#[async_trait]
impl Driver for Fixture {
    async fn open(&self, _: &Config) -> anyhow::Result<Box<dyn Session>> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            anyhow::bail!("fixture SSH failed")
        }
        Ok(Box::new(Live {
            fail: false,
            closes: self.closes.clone(),
        }))
    }
}
#[async_trait]
impl Session for Live {
    async fn health(&mut self) -> anyhow::Result<u64> {
        if self.fail {
            anyhow::bail!("fixture probe failed")
        }
        self.fail = true;
        Ok(5)
    }
    fn exited(&mut self) -> anyhow::Result<Option<String>> {
        Ok(None)
    }
    async fn close(&mut self) {
        self.closes.fetch_add(1, Ordering::SeqCst);
    }
}
struct Proxy {
    applies: AtomicUsize,
    restores: AtomicUsize,
}
impl ProxyOps for Proxy {
    fn apply(&self, _: u16) -> anyhow::Result<()> {
        self.applies.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn restore(&self) -> anyhow::Result<()> {
        self.restores.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn active(&self) -> anyhow::Result<bool> {
        Ok(false)
    }
}
fn port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
async fn controller(failed: bool) -> (tempfile::TempDir, Controller, Arc<Fixture>, Arc<Proxy>) {
    let d = tempfile::tempdir().unwrap();
    let key = d.path().join("key");
    std::fs::write(&key, "fixture").unwrap();
    let fixture = Arc::new(Fixture {
        opens: AtomicUsize::new(0),
        fail: AtomicBool::new(failed),
        closes: Arc::new(AtomicUsize::new(0)),
    });
    let proxy = Arc::new(Proxy {
        applies: AtomicUsize::new(0),
        restores: AtomicUsize::new(0),
    });
    let timing = Timing {
        health: Duration::from_millis(60),
        tick: Duration::from_millis(5),
        backoff_unit: Duration::from_millis(10),
    };
    let c = Controller::new_with(d.path().into(), fixture.clone(), proxy.clone(), timing).unwrap();
    c.save(Config {
        host: "example.com".into(),
        username: "user".into(),
        key_path: key.to_string_lossy().into(),
        socks_port: port(),
        http_port: port(),
        system_proxy: true,
        ..Config::default()
    })
    .await
    .unwrap();
    (d, c, fixture, proxy)
}
async fn until(c: &Controller, f: impl Fn(&xproxy::model::Status) -> bool) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if f(&c.status().await) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn failures_retry_but_manual_off_cancels_backoff_and_restores() {
    let (_d, c, f, p) = controller(true).await;
    c.set_enabled(true).await.unwrap();
    until(&c, |s| s.attempt >= 2).await;
    assert!(c.status().await.desired_on);
    assert!(c.status().await.latency_ms.is_none());
    assert_eq!(p.applies.load(Ordering::SeqCst), 0);
    c.set_enabled(false).await.unwrap();
    let opens = f.opens.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(opens, f.opens.load(Ordering::SeqCst));
    assert!(matches!(c.status().await.phase, Phase::Off));
    assert!(p.restores.load(Ordering::SeqCst) >= 1);
}

#[tokio::test]
async fn health_failure_reconnects_and_duplicate_enable_does_not_duplicate_supervisor() {
    let (_d, c, f, p) = controller(false).await;
    c.set_enabled(true).await.unwrap();
    c.set_enabled(true).await.unwrap();
    until(&c, |s| matches!(s.phase, Phase::Connected)).await;
    assert_eq!(f.opens.load(Ordering::SeqCst), 1);
    assert_eq!(c.status().await.latency_ms, Some(5));
    assert_eq!(p.applies.load(Ordering::SeqCst), 1);
    until(&c, |_| f.opens.load(Ordering::SeqCst) >= 2).await;
    assert!(f.closes.load(Ordering::SeqCst) >= 1);
    c.set_enabled(false).await.unwrap();
    assert!(!c.status().await.desired_on);
    assert!(c.status().await.latency_ms.is_none());
}

#[tokio::test]
async fn cannot_save_config_while_connecting() {
    let (_d, c, _f, _p) = controller(true).await;
    c.set_enabled(true).await.unwrap();
    assert!(c.save(Config::default()).await.is_err());
    c.set_enabled(false).await.unwrap();
}

struct Authorization {
    denied: AtomicBool,
    attempts: AtomicUsize,
}
impl ProxyOps for Authorization {
    fn apply(&self, _: u16) -> anyhow::Result<()> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        if self.denied.load(Ordering::SeqCst) {
            anyhow::bail!("denied")
        }
        Ok(())
    }
    fn restore(&self) -> anyhow::Result<()> {
        Ok(())
    }
    fn active(&self) -> anyhow::Result<bool> {
        Ok(!self.denied.load(Ordering::SeqCst))
    }
    fn pending_recovery(&self) -> anyhow::Result<bool> {
        Ok(false)
    }
}
#[tokio::test]
async fn system_proxy_retry_works_without_waiting_for_tunnel_failure() {
    let (d, c, f, _) = controller(false).await;
    let config = c.config().await;
    drop(c);
    let p = Arc::new(Authorization {
        denied: AtomicBool::new(true),
        attempts: AtomicUsize::new(0),
    });
    let c = Controller::new_with(
        d.path().into(),
        f.clone(),
        p.clone(),
        Timing {
            health: Duration::from_secs(60),
            ..Timing::default()
        },
    )
    .unwrap();
    c.save(config).await.unwrap();
    c.set_enabled(true).await.unwrap();
    until(&c, |s| {
        matches!(s.phase, Phase::Connected) && s.proxy_error.is_some()
    })
    .await;
    p.denied.store(false, Ordering::SeqCst);
    c.retry_now().await.unwrap();
    until(&c, |s| s.system_proxy_active && s.proxy_error.is_none()).await;
    assert_eq!(f.opens.load(Ordering::SeqCst), 1);
    assert_eq!(p.attempts.load(Ordering::SeqCst), 2);
    c.set_enabled(false).await.unwrap();
}

struct Hung;
#[async_trait]
impl Driver for Hung {
    async fn open(&self, _: &Config) -> anyhow::Result<Box<dyn Session>> {
        std::future::pending().await
    }
}
#[tokio::test]
async fn manual_off_cancels_inflight_handshake() {
    let (d, c, _f, p) = controller(false).await;
    let config = c.config().await;
    drop(c);
    let c = Controller::new_with(
        d.path().into(),
        Arc::new(Hung),
        p.clone(),
        Timing::default(),
    )
    .unwrap();
    c.save(config).await.unwrap();
    c.set_enabled(true).await.unwrap();
    until(&c, |s| s.attempt == 1).await;
    tokio::time::timeout(Duration::from_millis(200), c.set_enabled(false))
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(c.status().await.phase, Phase::Off));
    assert!(!c.status().await.desired_on);
}

struct ProcessExit;
struct Exited;
#[async_trait]
impl Driver for ProcessExit {
    async fn open(&self, _: &Config) -> anyhow::Result<Box<dyn Session>> {
        Ok(Box::new(Exited))
    }
}
#[async_trait]
impl Session for Exited {
    async fn health(&mut self) -> anyhow::Result<u64> {
        Ok(3)
    }
    fn exited(&mut self) -> anyhow::Result<Option<String>> {
        Ok(Some("fixture process exited".into()))
    }
    async fn close(&mut self) {}
}
#[tokio::test]
async fn process_exit_retries_without_waiting_for_probe_interval() {
    let (d, c, _f, p) = controller(false).await;
    let config = c.config().await;
    drop(c);
    let c = Controller::new_with(
        d.path().into(),
        Arc::new(ProcessExit),
        p,
        Timing {
            health: Duration::from_secs(60),
            tick: Duration::from_millis(5),
            backoff_unit: Duration::from_millis(5),
        },
    )
    .unwrap();
    c.save(config).await.unwrap();
    c.set_enabled(true).await.unwrap();
    until(&c, |s| s.attempt >= 2).await;
    assert!(c.status().await.desired_on);
    c.set_enabled(false).await.unwrap();
}

struct ConcurrentProxy {
    calls: AtomicUsize,
    inflight: AtomicUsize,
    max: AtomicUsize,
    live: AtomicBool,
    entered: tokio::sync::Notify,
    release: std::sync::Mutex<bool>,
    condition: std::sync::Condvar,
}
impl ProxyOps for ConcurrentProxy {
    fn apply(&self, _: u16) -> anyhow::Result<()> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if call == 1 {
            anyhow::bail!("fixture authorization denied")
        }
        let concurrent = self.inflight.fetch_add(1, Ordering::SeqCst) + 1;
        self.max.fetch_max(concurrent, Ordering::SeqCst);
        if call == 2 {
            self.entered.notify_one();
            let mut done = self.release.lock().unwrap();
            while !*done {
                done = self.condition.wait(done).unwrap();
            }
        }
        self.live.store(true, Ordering::SeqCst);
        self.inflight.fetch_sub(1, Ordering::SeqCst);
        Ok(())
    }
    fn restore(&self) -> anyhow::Result<()> {
        self.live.store(false, Ordering::SeqCst);
        Ok(())
    }
    fn active(&self) -> anyhow::Result<bool> {
        Ok(self.live.load(Ordering::SeqCst))
    }
    fn pending_recovery(&self) -> anyhow::Result<bool> {
        self.active()
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_proxy_retry_and_reconnection_serialize_mutations() {
    let (d, c, f, _) = controller(false).await;
    let config = c.config().await;
    drop(c);
    let proxy = Arc::new(ConcurrentProxy {
        calls: AtomicUsize::new(0),
        inflight: AtomicUsize::new(0),
        max: AtomicUsize::new(0),
        live: AtomicBool::new(false),
        entered: tokio::sync::Notify::new(),
        release: std::sync::Mutex::new(false),
        condition: std::sync::Condvar::new(),
    });
    let c = Arc::new(
        Controller::new_with(
            d.path().into(),
            f,
            proxy.clone(),
            Timing {
                health: Duration::from_millis(50),
                tick: Duration::from_millis(5),
                backoff_unit: Duration::from_millis(5),
            },
        )
        .unwrap(),
    );
    c.save(config).await.unwrap();
    c.set_enabled(true).await.unwrap();
    until(&c, |s| matches!(s.phase, Phase::Connected)).await;
    let retry_controller = c.clone();
    let retry = tokio::spawn(async move { retry_controller.retry_now().await });
    tokio::time::timeout(Duration::from_secs(1), proxy.entered.notified())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(120)).await;
    *proxy.release.lock().unwrap() = true;
    proxy.condition.notify_one();
    retry.await.unwrap().unwrap();
    c.set_enabled(false).await.unwrap();
    assert_eq!(proxy.max.load(Ordering::SeqCst), 1);
    assert!(!proxy.live.load(Ordering::SeqCst));
}
