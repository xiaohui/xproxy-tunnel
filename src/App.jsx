import React, { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
  House,
  Gear,
  Info,
  Desktop,
  Globe,
  ShareNetwork,
  ArrowsClockwise,
  ArrowSquareOut,
  ArrowRight,
  ArrowLeft,
  Circle,
  CheckCircle,
} from "@phosphor-icons/react";
import {
  defaultConfig,
  offStatus,
  statusView,
  formatMetric,
  validateConfig,
  acceptsPoll,
  canAdvance,
  useFixture,
} from "./model.mjs";
const native = !!window.__TAURI_INTERNALS__;
const preview = new URLSearchParams(location.search).get("preview");
const fixture = useFixture(native, preview);
function Switch({ on, onChange, disabled, label }) {
  return (
    <button
      type="button"
      role="switch"
      aria-label={label}
      aria-checked={on}
      disabled={disabled}
      className={`switch ${on ? "on" : ""}`}
      onClick={() => onChange(!on)}
    >
      <span />
    </button>
  );
}
function Icon({ as: Component }) {
  return (
    <span className="icon-circle">
      <Component size={30} weight="regular" />
    </span>
  );
}
export default function App() {
  const [page, setPage] = useState(
      preview === "settings" ? "settings" : "home",
    ),
    [step, setStep] = useState(1),
    [config, setConfig] = useState(
      fixture
        ? {
            ...defaultConfig,
            host: "203.0.113.10",
            username: "ubuntu",
            key_path: "/Users/demo/.ssh/id_ed25519",
            system_proxy: true,
          }
        : defaultConfig,
    ),
    [status, setStatus] = useState(
      fixture
        ? {
            ...offStatus,
            phase: "connected",
            desired_on: true,
            system_proxy_active: true,
            connect_ms: 1200,
            latency_ms: 86,
          }
        : offStatus,
    ),
    [busy, setBusy] = useState(false),
    [loaded, setLoaded] = useState(!native),
    [saved, setSaved] = useState(false),
    [error, setError] = useState(""),
    [notice, setNotice] = useState("");
  const generation = useRef(0),
    alive = useRef(true),
    mutating = useRef(false);
  const v = statusView(status),
    locked = status.desired_on || status.phase === "stopping";
  useEffect(() => {
    alive.current = true;
    let pending = false;
    async function poll() {
      if (pending || !native || mutating.current) return;
      pending = true;
      const g = generation.current;
      try {
        const s = await invoke("get_status");
        if (acceptsPoll(g, generation.current, alive.current)) setStatus(s);
      } catch (e) {
        if (acceptsPoll(g, generation.current, alive.current))
          setError(String(e));
      } finally {
        pending = false;
      }
    }
    const initial = generation.current;
    if (native)
      invoke("get_config")
        .then((c) => {
          if (acceptsPoll(initial, generation.current, alive.current)) {
            setConfig(c);
            setLoaded(true);
            setSaved(!validateConfig(c));
          }
        })
        .catch((e) => {
          if (acceptsPoll(initial, generation.current, alive.current)) {
            setLoaded(false);
            setError(`无法读取配置：${String(e)}`);
          }
        });
    poll();
    const timer = setInterval(poll, 1000);
    return () => {
      alive.current = false;
      ++generation.current;
      clearInterval(timer);
    };
  }, []);
  async function action(fn) {
    setError("");
    setNotice("");
    if (!native) {
      setError(
        "浏览器预览无法操作 SSH、系统代理或本地浏览器。请在 XProxy 桌面应用中使用。",
      );
      return;
    }
    if (!loaded) {
      setError("配置尚未读取成功，请重新打开应用后再试。");
      return;
    }
    mutating.current = true;
    ++generation.current;
    setBusy(true);
    try {
      await fn();
      const s = await invoke("get_status");
      if (alive.current) setStatus(s);
    } catch (e) {
      if (alive.current) setError(String(e));
    } finally {
      mutating.current = false;
      if (alive.current) setBusy(false);
    }
  }
  const update = (key, value) => {
    setSaved(false);
    setConfig((c) => ({ ...c, [key]: value }));
  };
  async function save(connect) {
    const message = validateConfig(config);
    if (message) {
      setError(message);
      return;
    }
    await action(async () => {
      const normalized = {
        ...config,
        host: config.host.trim(),
        username: config.username.trim(),
        ssh_port: Number(config.ssh_port),
        socks_port: Number(config.socks_port),
        http_port: Number(config.http_port),
      };
      setConfig(await invoke("save_config", { config: normalized }));
      setSaved(true);
      if (connect) {
        await invoke("set_enabled", { enabled: true });
        setPage("home");
      } else setStep(2);
    });
  }
  function field(label, key, hint, type = "text") {
    return (
      <div className="field">
        <label htmlFor={key}>{label}</label>
        <div>
          <input
            id={key}
            type={type}
            min={
              type === "number" ? (key === "ssh_port" ? 1 : 1024) : undefined
            }
            max={type === "number" ? 65535 : undefined}
            value={config[key]}
            disabled={locked || busy}
            onChange={(e) =>
              update(
                key,
                type === "number" ? Number(e.target.value) : e.target.value,
              )
            }
          />
          {hint && <p>{hint}</p>}
        </div>
      </div>
    );
  }
  return (
    <div className="app-shell">
      <header className="app-header">
        <button className="brand" onClick={() => setPage("home")}>
          XProxy
        </button>
        {page === "settings" ? (
          <button className="header-home" onClick={() => setPage("home")}>
            <House size={18} />
            首页
          </button>
        ) : (
          <span>安全连接 · 本地代理</span>
        )}
      </header>
      {!native && (
        <div className="preview-banner">
          {fixture
            ? "设计预览 · 示例连接与指标，未建立真实 SSH 连接"
            : "浏览器预览 · SSH 与系统代理操作仅在桌面应用中可用"}
        </div>
      )}
      <div className="app-body">
        {page === "home" && (
          <nav className="sidebar">
            <button
              className={page === "home" ? "selected" : ""}
              onClick={() => setPage("home")}
            >
              <House size={25} weight={page === "home" ? "fill" : "regular"} />
              首页
            </button>
            <button
              className={page === "settings" ? "selected" : ""}
              onClick={() => setPage("settings")}
            >
              <Gear size={25} />
              配置
            </button>
            <div className="sidebar-bottom">
              <button
                onClick={() =>
                  setNotice(
                    "XProxy 0.1.0 · 通过 SSH 提供本地 SOCKS5 与 HTTP 代理。",
                  )
                }
              >
                <Info size={23} />
                关于
              </button>
              <small>v0.1.0</small>
            </div>
          </nav>
        )}
        <main className={page === "settings" ? "settings" : "home"}>
          {(error || notice) && (
            <div
              role={error ? "alert" : "status"}
              className={`message ${error ? "error" : ""}`}
            >
              {error || notice}
            </div>
          )}
          {page === "home" ? (
            <>
              <section className="status-hero">
                <div className="status-title">
                  <Circle
                    size={48}
                    weight="fill"
                    className={
                      v.connected
                        ? "green"
                        : status.desired_on
                          ? "blue"
                          : "muted"
                    }
                  />
                  <div>
                    <h1>{v.label}</h1>
                    <p>
                      {v.connected
                        ? `经 SSH 隧道连接至 ${config.host}`
                        : status.desired_on
                          ? `正在连接至 ${config.host} · 第 ${status.attempt || 1} 次尝试`
                          : "配置 SSH 服务器，开启安全代理连接。"}
                    </p>
                  </div>
                </div>
                <div className="main-control">
                  <div>
                    <label>代理开关</label>
                    <Switch
                      label="代理开关"
                      on={v.on}
                      disabled={busy || !loaded || status.phase === "stopping"}
                      onChange={(enabled) =>
                        enabled && !config.host
                          ? (setPage("settings"),
                            setError("请先完成 SSH 配置。"))
                          : action(() => invoke("set_enabled", { enabled }))
                      }
                    />
                  </div>
                  <p>{v.on ? "已开启" : "已关闭"}</p>
                </div>
              </section>
              {(status.last_error || status.proxy_error) && (
                <div className="message error" role="alert">
                  {status.last_error || status.proxy_error}
                  {status.desired_on &&
                    (!v.connected ||
                      (status.proxy_error && !status.system_proxy_active)) && (
                      <button
                        onClick={() => action(() => invoke("retry_now"))}
                        disabled={busy}
                      >
                        {v.connected ? "重试系统代理" : "立即重试"}
                      </button>
                    )}
                </div>
              )}
              <section className="metrics">
                <div>
                  <p>建立连接用时</p>
                  <strong>
                    {formatMetric(
                      v.connected ? status.connect_ms : null,
                      "connect",
                    )}
                  </strong>
                </div>
                <div>
                  <p>代理探测延迟</p>
                  <strong>
                    {formatMetric(
                      v.connected ? status.latency_ms : null,
                      "latency",
                    )}
                  </strong>
                  {v.connected && status.latency_ms != null && (
                    <span className="badge">
                      {fixture ? "示例状态：正常" : "探测成功"}
                    </span>
                  )}
                </div>
              </section>
              <section className="scope">
                <h2>代理范围</h2>
                <p className="section-subtitle">
                  {v.connected
                    ? "代理已就绪，可按以下方式使用。"
                    : "连接成功后，可在系统和独立浏览器中使用。"}
                </p>
                <div className="scope-grid">
                  <div className="scope-item">
                    <Icon as={Desktop} />
                    <div>
                      <h3>
                        系统代理{" "}
                        <span
                          className={
                            status.system_proxy_active ? "green" : "muted"
                          }
                        >
                          {status.system_proxy_active ? (
                            <>
                              <Circle weight="fill" size={14} /> 已启用
                            </>
                          ) : (
                            "未启用"
                          )}
                        </span>
                      </h3>
                      <p>
                        {status.system_proxy_active
                          ? "遵循系统代理设置的应用将使用代理。"
                          : "可在配置中启用，仅影响遵循系统代理设置的应用。"}
                      </p>
                    </div>
                  </div>
                  <div className="scope-item">
                    <Icon as={Globe} />
                    <div>
                      <h3>浏览器代理</h3>
                      <p>使用独立配置的 Chrome 或 Edge 浏览器。</p>
                      <button
                        className="primary browser-button"
                        disabled={!v.connected || busy}
                        onClick={() =>
                          action(async () => {
                            const name = await invoke("launch_browser");
                            setNotice(`已启动 ${name} 独立代理浏览器。`);
                          })
                        }
                      >
                        <ArrowSquareOut size={21} />
                        启动独立浏览器
                      </button>
                    </div>
                  </div>
                </div>
              </section>
              <section className="detail-row">
                <Icon as={ShareNetwork} />
                <div>
                  <h3>本地代理</h3>
                  <p>
                    SOCKS5 代理
                    <br />
                    <span>127.0.0.1:{config.socks_port}</span>
                  </p>
                  <p className="http-endpoint">
                    HTTP 代理 · 127.0.0.1:{config.http_port}
                  </p>
                </div>
              </section>
              <section className="detail-row reconnect">
                <Icon as={ArrowsClockwise} />
                <div>
                  <h3>
                    自动重连{" "}
                    <span className="green">
                      <CheckCircle weight="fill" size={17} />
                      已开启
                    </span>
                  </h3>
                  <p>连接中断时自动重试，手动关闭后停止。</p>
                  {status.retry_at_ms && status.desired_on && (
                    <p>
                      下次尝试：
                      {new Date(status.retry_at_ms).toLocaleTimeString("zh-CN")}
                    </p>
                  )}
                </div>
              </section>
            </>
          ) : (
            <>
              <h1>引导式设置</h1>
              <p className="settings-subtitle">
                通过 SSH 建立安全连接，并将本地流量通过远程服务器转发。
              </p>
              <div className="steps">
                <button
                  className={step === 1 ? "active" : ""}
                  onClick={() => setStep(1)}
                >
                  <span>1</span>SSH 连接
                </button>
                <div />
                <button
                  className={step === 2 ? "active" : ""}
                  disabled={!canAdvance(config, saved)}
                  onClick={() => setStep(2)}
                >
                  <span>2</span>代理范围
                </button>
              </div>
              <div className="settings-grid">
                <form
                  onSubmit={(e) => {
                    e.preventDefault();
                    save(step === 2);
                  }}
                >
                  <fieldset disabled={locked || busy || !loaded}>
                    {step === 1 ? (
                      <>
                        <h2 className="ruled">SSH 服务器配置</h2>
                        {field(
                          "服务器 IP",
                          "host",
                          "输入远程服务器的 IP 地址或域名。",
                        )}
                        {field(
                          "SSH 用户名",
                          "username",
                          "登录远程服务器的用户名。",
                        )}
                        {field(
                          "端口",
                          "ssh_port",
                          "SSH 服务端口，默认 22。",
                          "number",
                        )}
                        <div className="field">
                          <label htmlFor="key_path">私钥文件</label>
                          <div>
                            <div className="file-input">
                              <input
                                id="key_path"
                                value={config.key_path}
                                onChange={(e) =>
                                  update("key_path", e.target.value)
                                }
                              />
                              <button
                                type="button"
                                onClick={() =>
                                  action(async () => {
                                    const path = await open({
                                      multiple: false,
                                      directory: false,
                                      title: "选择 SSH 私钥文件",
                                    });
                                    if (typeof path === "string")
                                      update("key_path", path);
                                  })
                                }
                              >
                                选择文件…
                              </button>
                            </div>
                            <p>选择本地 SSH 私钥文件的完整绝对路径。</p>
                          </div>
                        </div>
                        <h2 className="ruled local-title">本地代理设置</h2>
                        <div className="field">
                          <label htmlFor="socks_port">本地 SOCKS5</label>
                          <div>
                            <div className="endpoint">
                              <input
                                aria-label="本地绑定地址"
                                value="127.0.0.1"
                                readOnly
                              />
                              <span>:</span>
                              <input
                                id="socks_port"
                                type="number"
                                min="1024"
                                max="65535"
                                value={config.socks_port}
                                onChange={(e) =>
                                  update("socks_port", Number(e.target.value))
                                }
                              />
                            </div>
                            <p>
                              XProxy 将在本地启动 SOCKS5
                              代理，用于转发您的网络流量。
                            </p>
                          </div>
                        </div>
                        <details>
                          <summary>高级设置</summary>
                          {field(
                            "HTTP 端口",
                            "http_port",
                            "本地 HTTP 代理端口，默认 1081。",
                            "number",
                          )}
                          {field(
                            "探测地址",
                            "probe_url",
                            "通过无用户名和密码的 HTTPS 地址验证代理连通性。",
                          )}
                        </details>
                      </>
                    ) : (
                      <>
                        <h2 className="ruled">选择代理范围</h2>
                        <div className="scope-choice">
                          <Icon as={Desktop} />
                          <div>
                            <h3>系统代理</h3>
                            <p>
                              连接后自动设置系统代理。仅影响遵循系统代理设置的应用。
                            </p>
                          </div>
                          <Switch
                            label="启用系统代理"
                            on={config.system_proxy}
                            disabled={locked || busy}
                            onChange={(value) => update("system_proxy", value)}
                          />
                        </div>
                        <div className="scope-choice">
                          <Icon as={Globe} />
                          <div>
                            <h3>独立代理浏览器</h3>
                            <p>
                              连接成功后，从首页启动独立配置的 Chrome 或
                              Edge，无需启用系统代理。
                            </p>
                          </div>
                        </div>
                        <div className="scope-choice">
                          <Icon as={ShareNetwork} />
                          <div>
                            <h3>本地代理连接成功后可用</h3>
                            <p>
                              SOCKS5 · 127.0.0.1:{config.socks_port}
                              <br />
                              HTTP · 127.0.0.1:{config.http_port}
                            </p>
                          </div>
                        </div>
                        <button
                          className="text-button"
                          type="button"
                          onClick={() => setStep(1)}
                        >
                          <ArrowLeft size={18} />
                          返回 SSH 配置
                        </button>
                      </>
                    )}
                  </fieldset>
                  {locked ? (
                    <div className="settings-locked">
                      <p>修改配置前，请先关闭当前连接。</p>
                      <button
                        className="secondary"
                        type="button"
                        disabled={busy || status.phase === "stopping"}
                        onClick={() =>
                          action(() =>
                            invoke("set_enabled", { enabled: false }),
                          )
                        }
                      >
                        关闭连接以编辑
                      </button>
                    </div>
                  ) : (
                    <button
                      className="primary continue"
                      type="submit"
                      disabled={busy || !loaded}
                    >
                      {busy
                        ? "正在保存…"
                        : step === 1
                          ? "保存并继续"
                          : "保存并连接"}
                      <ArrowRight size={23} />
                    </button>
                  )}
                </form>
                <aside>
                  <h2 className="ruled">当前状态</h2>
                  <h3 className="aside-status">
                    <Circle
                      weight="fill"
                      size={20}
                      className={v.connected ? "green" : "muted"}
                    />
                    {v.label}
                  </h3>
                  <p>
                    {v.connected
                      ? `已连接至 ${config.host}`
                      : "请完成左侧配置并连接。"}
                  </p>
                  <div className="aside-next">
                    <h3>下一步</h3>
                    <p>保存 SSH 配置后，您可以选择：</p>
                    <ul>
                      <li>
                        <strong>配置系统代理</strong>
                        <p>
                          将系统网络代理设置为 XProxy，影响遵循代理设置的应用。
                        </p>
                      </li>
                      <li>
                        <strong>启动独立代理浏览器</strong>
                        <p>
                          使用独立配置的 Chrome 或 Edge
                          访问互联网，无需修改系统代理。
                        </p>
                      </li>
                    </ul>
                  </div>
                </aside>
              </div>
            </>
          )}
        </main>
      </div>
    </div>
  );
}
