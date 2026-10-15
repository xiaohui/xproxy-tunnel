export const defaultConfig = {
  host: "",
  username: "",
  ssh_port: 22,
  key_path: "",
  socks_port: 1080,
  http_port: 1081,
  system_proxy: false,
  probe_url: "https://www.gstatic.com/generate_204",
};
export const offStatus = {
  desired_on: false,
  phase: "off",
  attempt: 0,
  retry_at_ms: null,
  connected_at_ms: null,
  connect_ms: null,
  latency_ms: null,
  last_error: null,
  system_proxy_active: false,
  proxy_error: null,
};
export function statusView(s) {
  return {
    on: !!s.desired_on,
    connected: s.phase === "connected",
    label:
      {
        off: "未连接",
        connecting: "正在连接",
        connected: "已连接",
        reconnecting: "正在重连",
        stopping: "正在停止",
        error: "连接失败",
      }[s.phase] || "未知状态",
  };
}
export function formatMetric(n, type) {
  return n == null
    ? "—"
    : type === "connect"
      ? `${(n / 1000).toFixed(1)} 秒`
      : `${Math.round(n)} 毫秒`;
}
export function acceptsPoll(g, current, alive) {
  return alive && g === current;
}
export function validHost(host) {
  const dns =
    host.length > 0 &&
    host.length <= 253 &&
    host
      .split(".")
      .every(
        (label) =>
          label.length > 0 &&
          label.length <= 63 &&
          /^[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?$/.test(label),
      );
  if (dns) return true;
  if (!host.includes(":") || /[\s\[\]\/%]/.test(host)) return false;
  try {
    return new URL(`http://[${host}]/`).hostname.startsWith("[");
  } catch {
    return false;
  }
}
export function validateConfig(c) {
  if (!validHost(c.host)) return "请输入有效的服务器 IP 或域名。";
  if (!/^[A-Za-z0-9._][A-Za-z0-9._-]{0,127}$/.test(c.username))
    return "SSH 用户名只能包含字母、数字、点、下划线和连字符，且不能以连字符开头。";
  if (
    !c.key_path ||
    /[\r\n\0]/.test(c.key_path) ||
    !/^(?:\/|[A-Za-z]:[\\/]|\\\\)/.test(c.key_path)
  )
    return "请选择 SSH 私钥文件（绝对路径）。";
  if (
    !Number.isInteger(Number(c.ssh_port)) ||
    Number(c.ssh_port) < 1 ||
    Number(c.ssh_port) > 65535
  )
    return "SSH 端口必须是 1–65535 的整数。";
  for (const p of [c.socks_port, c.http_port])
    if (!Number.isInteger(Number(p)) || Number(p) < 1024 || Number(p) > 65535)
      return "本地端口必须是 1024–65535 的整数。";
  if (Number(c.socks_port) === Number(c.http_port))
    return "SOCKS5 和 HTTP 端口必须不同。";
  try {
    const u = new URL(c.probe_url);
    if (u.protocol !== "https:" || !u.hostname || u.username || u.password)
      throw Error();
  } catch {
    return "连接探测地址必须是无用户名和密码的 HTTPS URL。";
  }
  return "";
}

export function canAdvance(config, saved) {
  return !!saved && !validateConfig(config);
}
export function useFixture(native, preview) {
  return !native && preview === "connected";
}
