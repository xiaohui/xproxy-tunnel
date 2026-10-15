# XProxy Tunnel 0.1.0

Tauri 2 + Rust desktop SSH proxy for macOS and Windows. The homepage follows the selected status-first design; configuration is a two-step SSH / proxy-scope flow.

## Start

Requirements: Node.js 20.19+, Rust 1.90+, the platform's Tauri prerequisites, and OpenSSH Client. Windows also requires Visual Studio C++ build tools and WebView2. Enable Windows' optional **OpenSSH Client** feature if `C:\Windows\System32\OpenSSH\ssh.exe` is missing. macOS uses `/usr/bin/ssh`.

```sh
cd /Users/bytedance/xiaohui/xproxy/desktop
npm ci --cache .cache/npm
npm run desktop:dev
```

Configure the server IP/domain, SSH username, SSH port and absolute private-key path. Local SOCKS5 defaults to `127.0.0.1:1080`; the HTTP/HTTPS proxy bridge defaults to `127.0.0.1:1081`. Both ports must be at least 1024. In the second step, choose whether to apply system proxy settings, then save and connect.

The private key is never copied into app data. Encrypted keys must already be unlocked in `ssh-agent`. Authentication runs in batch mode: password/passphrase prompts inside the SSH process are deliberately disabled. The app maintains its own `known_hosts`: first use trusts the presented key; changed keys are rejected. If a server key changes legitimately, verify its new fingerprint independently before updating the application's `known_hosts`.

Closing the main window hides it to the system tray and keeps the tunnel running. Use the tray menu's **打开 XProxy Tunnel** action to show the window again, or **退出** to stop the tunnel and restore system proxy settings before the process exits.

## Behavior

- The homepage switch expresses the desired state. Actual states are connecting, connected, reconnecting, stopping and off.
- Connected means a successful HTTPS request through the SSH SOCKS5 tunnel, not just a listening localhost port. The default probe is `https://www.gstatic.com/generate_204`, configurable under Advanced settings. It must return 2xx; redirects, TLS failures and timeouts fail health checks.
- Health checks run every 10 seconds; OpenSSH keepalives run every 10 seconds and stop after two unanswered messages. Process exit is checked every 200 ms. A failed probe has an 8-second timeout. Detection is periodic, not instantaneous.
- Failures retry after 1, 2, 4, 8, 16 and then 30 seconds. Manual off cancels pending authentication, forwarding and retries. Establishment time and probe latency remain unknown until confirmed; a lost connection clears live measurements.
- System proxy mode uses the local HTTP/CONNECT bridge. macOS applies to enabled network services, using native authorization when required. Windows applies to the current user's default LAN WinINet connection. These settings affect applications that honor system proxies; this is not a VPN/TUN or all-traffic interception.
- Browser mode launches an installed Chrome or Edge with a separate local profile, SOCKS5, remote hostname resolution and QUIC disabled. It is an external browser window, not Tauri's embedded WebView. Firefox and modification of existing browser profiles are outside v0.1.
- Original system settings are durably recorded before mutation. Stop/quit restores app-owned changes. Startup recovers an interrupted previous session. Externally changed properties are preserved. Unresolved restoration keeps its journal; normal quit waits for cleanup and stays open if recovery still needs attention.
- During reconnect, the HTTP bridge rejects new requests and closes existing bridge tunnels. System proxy settings remain pointed at the bridge until manual off. This does not prevent applications that ignore system proxies from connecting directly.
- SSH runs under a pipe-based guardian. Parent death closes the lease and terminates SSH. Normal shutdown also reaps the guardian. The application uses a per-user instance lock to prevent concurrent sessions modifying proxy snapshots.

App data defaults to `~/Library/Application Support/dev.xproxy.tunnel` on macOS and Tauri's per-user app-data directory on Windows. `XPROXY_DATA_DIR` can isolate development runs. Stored files include configuration paths, known host keys, the instance lock, separate browser profiles and an outstanding system-proxy recovery journal. No SSH password or private-key contents are stored.

## Build

```sh
# macOS local development app; unsigned
npm run tauri -- build --debug --bundles app --no-sign
# macOS DMG installer
npm run tauri -- build --bundles dmg
# Windows NSIS installer (run on Windows)
npm run tauri -- build --bundles nsis
```

Current macOS development output:
`src-tauri/target/debug/bundle/macos/XProxy Tunnel.app`.

GitHub Actions runs checks on macOS and Windows for pushes and pull requests. Pushing a `v*` tag builds Apple Silicon and Intel macOS DMGs plus a Windows x64 NSIS installer, then creates a GitHub Draft Release. Review and publish the draft after confirming the files. Packages are currently unsigned; macOS signing/notarization and Windows code signing need publisher credentials before broad public distribution.

## Verification

```sh
node --test tests/frontend.test.mjs tests/sites-worker.test.mjs
cargo test --manifest-path src-tauri/Cargo.toml --no-default-features
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
python3 tests/guardian_test.py
npm run build
```

The guardian tests require the debug desktop binary. Core fixtures do not modify real system networking. See `verification.md` for the actual acceptance boundary and `design-qa.md` for the blocked visual comparison.

To check a real server without applying system proxy settings, supply a test config to the developer runner:

```sh
cargo run --manifest-path src-tauri/Cargo.toml --no-default-features \
  --example tunnel_check -- /absolute/path/test-config.json /absolute/path/isolated-test-data
```

It reports actual status, requires a verified HTTPS probe and stops the tunnel before returning. Use ports that are not occupied.

## References

- [Tauri project setup](https://v2.tauri.app/start/create-project/)
- [Tauri native file dialog](https://v2.tauri.app/plugin/dialog/)
- [OpenSSH configuration / dynamic forwarding](https://man.openbsd.org/ssh_config)
- [Chromium SOCKS proxy and remote hostname resolution](https://chromium.googlesource.com/website/+/refs/heads/main/site/developers/design-documents/network-stack/socks-proxy/index.md)
- [Windows per-connection internet options](https://learn.microsoft.com/en-us/windows/win32/api/wininet/ns-wininet-internet_per_conn_option_listw)

The included `.github/workflows/desktop-check.yml` verifies macOS and Windows when this folder becomes the repository root. It has not been run or published from this checkout.
