use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub host: String,
    pub username: String,
    pub ssh_port: u16,
    pub key_path: String,
    pub socks_port: u16,
    pub http_port: u16,
    pub system_proxy: bool,
    pub probe_url: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: String::new(),
            username: String::new(),
            ssh_port: 22,
            key_path: String::new(),
            socks_port: 1080,
            http_port: 1081,
            system_proxy: false,
            probe_url: "https://www.gstatic.com/generate_204".into(),
        }
    }
}

impl Config {
    pub fn validate(&self) -> anyhow::Result<()> {
        let ip = self.host.parse::<std::net::IpAddr>().is_ok();
        let dns = !self.host.is_empty()
            && self.host.len() <= 253
            && self.host.split('.').all(|s| {
                !s.is_empty()
                    && s.len() <= 63
                    && !s.starts_with('-')
                    && !s.ends_with('-')
                    && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            });
        anyhow::ensure!(ip || dns, "请输入有效的服务器 IP 或域名");
        anyhow::ensure!(
            !self.username.is_empty()
                && self.username.len() <= 128
                && self
                    .username
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                && !self.username.starts_with('-'),
            "SSH 用户名只能包含字母、数字、点、下划线和连字符"
        );
        anyhow::ensure!(
            std::path::Path::new(&self.key_path).is_absolute()
                && std::path::Path::new(&self.key_path).is_file(),
            "请选择存在的 SSH 私钥文件（绝对路径）"
        );
        anyhow::ensure!(!self.key_path.contains(['\r', '\n', '\0']), "私钥路径无效");
        anyhow::ensure!(
            self.ssh_port != 0 && self.socks_port >= 1024 && self.http_port >= 1024,
            "本地端口应在 1024–65535，SSH 端口应在 1–65535"
        );
        anyhow::ensure!(
            self.socks_port != self.http_port,
            "SOCKS5 和 HTTP 代理端口不能相同"
        );
        let url = reqwest::Url::parse(&self.probe_url)?;
        anyhow::ensure!(
            url.scheme() == "https"
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none(),
            "连接探测地址必须是无用户名和密码的 HTTPS URL"
        );
        Ok(())
    }
}

pub fn retry_seconds(attempt: u32) -> u64 {
    (1_u64 << attempt.saturating_sub(1).min(5)).min(30)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Off,
    Connecting,
    Connected,
    Reconnecting,
    Stopping,
    Error,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Status {
    pub desired_on: bool,
    pub phase: Phase,
    pub attempt: u32,
    pub retry_at_ms: Option<u64>,
    pub connected_at_ms: Option<u64>,
    pub connect_ms: Option<u64>,
    pub latency_ms: Option<u64>,
    pub last_error: Option<String>,
    pub system_proxy_active: bool,
    pub proxy_error: Option<String>,
}

impl Status {
    pub fn failed(&mut self, error: String) {
        self.phase = if self.desired_on {
            Phase::Reconnecting
        } else {
            Phase::Off
        };
        self.latency_ms = None;
        self.connect_ms = None;
        self.connected_at_ms = None;
        self.last_error = Some(error);
    }
}
