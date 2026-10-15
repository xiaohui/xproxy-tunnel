//! Durable, compare-before-restore system proxy transaction.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
};
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(windows)]
mod windows;
pub trait Backend {
    fn snapshot(&self) -> Result<BTreeMap<String, Value>>;
    fn desired(
        &self,
        port: u16,
        original: &BTreeMap<String, Value>,
    ) -> Result<BTreeMap<String, Value>>;
    fn write(&self, key: &str, value: &Value) -> Result<()>;
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u8,
    original: BTreeMap<String, Value>,
    owned: BTreeMap<String, Value>,
}
pub struct Transaction<'a> {
    dir: PathBuf,
    backend: &'a dyn Backend,
}
impl<'a> Transaction<'a> {
    pub fn new(dir: PathBuf, backend: &'a dyn Backend) -> Self {
        Self { dir, backend }
    }
    fn path(&self) -> PathBuf {
        self.dir.join("system-proxy.json")
    }
    fn read(&self) -> Result<Journal> {
        let j: Journal = serde_json::from_slice(&fs::read(self.path())?)
            .context("Invalid proxy recovery journal; refusing to alter system settings")?;
        if j.version != 1 || j.original.keys().ne(j.owned.keys()) {
            bail!("Invalid proxy recovery journal schema")
        }
        Ok(j)
    }
    fn save(&self, j: &Journal) -> Result<()> {
        fs::create_dir_all(&self.dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.dir, fs::Permissions::from_mode(0o700))?;
        }
        let tmp = self.dir.join("system-proxy.json.tmp");
        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        // A previous interrupted temp write is never used as a recovery journal.
        if tmp.exists() {
            fs::remove_file(&tmp)?;
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(&serde_json::to_vec(j)?)?;
        f.sync_all()?;
        fs::rename(tmp, self.path())?;
        self.sync_dir()
    }
    fn sync_dir(&self) -> Result<()> {
        #[cfg(unix)]
        fs::File::open(&self.dir)?.sync_all()?;
        Ok(())
    }
    pub fn active(&self) -> Result<bool> {
        if self.path().exists() {
            let journal = self.read()?;
            let current = self.backend.snapshot()?;
            Ok(journal
                .owned
                .iter()
                .filter(|(k, v)| journal.original.get(*k) != Some(*v))
                .all(|(k, v)| current.get(k) == Some(v)))
        } else {
            Ok(false)
        }
    }
    pub fn apply(&self, port: u16) -> Result<()> {
        if port == 0 {
            bail!("System proxy port must be nonzero")
        }
        if self.path().exists() {
            self.restore()?;
        }
        let original = self.backend.snapshot()?;
        let owned = self.backend.desired(port, &original)?;
        if original.keys().ne(owned.keys()) {
            bail!("Backend returned inconsistent proxy keys")
        }
        let j = Journal {
            version: 1,
            original,
            owned,
        };
        self.save(&j)?;
        for (k, v) in &j.owned {
            if j.original.get(k) == Some(v) {
                continue;
            }
            if let Err(e) = self.backend.write(k, v) {
                let recovery = self.restore();
                return match recovery {
                    Ok(()) => {
                        Err(e.context("System proxy apply failed; original settings restored"))
                    }
                    Err(r) => Err(e.context(format!(
                        "System proxy apply failed; recovery requires attention: {r}"
                    ))),
                };
            }
        }
        let verification = self.backend.snapshot().and_then(|current| {
            if j.owned
                .iter()
                .filter(|(k, v)| j.original.get(*k) != Some(*v))
                .all(|(k, v)| current.get(k) == Some(v))
            {
                Ok(())
            } else {
                bail!("系统代理写入后核验失败")
            }
        });
        if let Err(e) = verification {
            let recovery = self.restore();
            return match recovery {
                Ok(()) => {
                    Err(e.context("System proxy readback failed; original settings restored"))
                }
                Err(r) => Err(e.context(format!(
                    "System proxy readback failed; recovery requires attention: {r}"
                ))),
            };
        }
        Ok(())
    }
    pub fn restore(&self) -> Result<()> {
        if !self.path().exists() {
            return Ok(());
        }
        let j = self.read()?;
        let current = self.backend.snapshot()?;
        let mut conflicts = 0;
        let mut failures = Vec::new();
        let mut restored = Vec::new();
        for (k, original) in &j.original {
            // Values we did not change are not owned by this transaction.
            if j.owned.get(k) == Some(original) {
                continue;
            }
            match current.get(k) {
                Some(v) if v == original => {}
                Some(v) if Some(v) == j.owned.get(k) => {
                    if let Err(e) = self.backend.write(k, original) {
                        failures.push(e.to_string());
                    } else {
                        restored.push(k);
                    }
                }
                _ => {
                    conflicts += 1;
                }
            }
        }
        if !failures.is_empty() {
            bail!(
                "System proxy restoration failed; recovery journal retained: {}",
                failures.join("; ")
            )
        }
        let verified = self
            .backend
            .snapshot()
            .context("System proxy restoration readback failed; recovery journal retained")?;
        if restored
            .iter()
            .any(|k| verified.get(*k) != j.original.get(*k))
        {
            bail!("系统代理恢复后核验失败，已保留恢复记录供重试")
        }
        fs::remove_file(self.path())?;
        self.sync_dir()?;
        if conflicts > 0 {
            bail!("Preserved {conflicts} externally changed system proxy properties")
        };
        Ok(())
    }
}
pub struct SystemProxy {
    pub dir: PathBuf,
    #[cfg(target_os = "macos")]
    native: macos::MacOS,
}
impl SystemProxy {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            #[cfg(target_os = "macos")]
            native: macos::MacOS::default(),
        }
    }
    pub fn apply(&self, port: u16) -> Result<()> {
        with_backend(self, |b| Transaction::new(self.dir.clone(), b).apply(port))
    }
    pub fn restore(&self) -> Result<()> {
        with_backend(self, |b| Transaction::new(self.dir.clone(), b).restore())
    }
    pub fn active(&self) -> Result<bool> {
        with_backend(self, |b| Transaction::new(self.dir.clone(), b).active())
    }
}
fn with_backend<T>(proxy: &SystemProxy, f: impl FnOnce(&dyn Backend) -> Result<T>) -> Result<T> {
    #[cfg(target_os = "macos")]
    {
        f(&proxy.native)
    }
    #[cfg(windows)]
    {
        let _ = proxy;
        f(&windows::Windows)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (f, proxy);
        bail!("System proxy integration supports macOS and Windows")
    }
}
