use anyhow::Context;
use serde::{de::DeserializeOwned, Serialize};
#[cfg(unix)]
use std::fs::File;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

pub fn private_dir(path: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    let parent = path.parent().context("missing data directory")?;
    private_dir(parent)?;
    let temp = path.with_extension("tmp");
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut f = options.open(&temp)?;
    f.write_all(&serde_json::to_vec_pretty(value)?)?;
    f.sync_all()?;
    drop(f);
    fs::rename(temp, path)?;
    #[cfg(unix)]
    {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> anyhow::Result<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(
            serde_json::from_slice(&bytes).context("本地配置文件损坏，请先备份后修复")?,
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
