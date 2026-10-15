# Verification receipt

Last checked: 2026-10-15, Asia/Shanghai.

## Passed

- Rust core: 29 tests. Config and argument validation; bounded retry; no duplicate supervisor; health/process failures; cancellation during authentication/backoff; system retry after denied authorization; stale metric clearing; repeated quit protection; proxy journal/readback/rollback/crash recovery; disabled service restoration and external-change preservation; real local SOCKS handshake, HTTP forwarding, CONNECT and in-flight tunnel cancellation.
- Frontend: 8 logic tests. Actual and desired state, unknown metrics, validation, navigation gating, fixture exclusion in native runtime and stale-response suppression.
- Starter packaging: 4 tests; preserved intentionally by the Product Design starter.
- Guardian: 2 actual subprocess tests against the compiled debug binary. Pipe EOF and abrupt parent exit both terminate the child. No process-list/screen APIs are needed for these tests.
- `cargo clippy --all-targets -- -D warnings`, including the native macOS platform adapter and Tauri executable.
- Frontend production build.
- Tauri native macOS debug bundle, unsigned: `src-tauri/target/debug/bundle/macos/XProxy Tunnel.app`, approximately 39.52 MiB at the checked build.

## Blocked / not established

- **Rendered UI / interaction comparison:** the in-app browser refused `http://127.0.0.1:4175` initially because origin approval was not granted. After the user explicitly granted local preview permission, a second attempt was still rejected: a saved permission or managed policy denies the origin. No alternate browser, raw CDP or other workaround was used. Source references are retained, but no implementation screenshots or visual pass are claimed.
- **Real SSH handshake/HTTPS end-to-end:** a local ephemeral OpenSSH server started listening, but its pre-authentication child failed with `sandbox initialization failed: Operation not permitted` and `ssh_sandbox_child: sandbox_init: Operation not permitted [preauth]`. Both the application runner and a direct `/usr/bin/ssh` client received connection reset. This is an environmental blocker, not a successful tunnel test. No user's remote credentials were supplied.
- **Live system settings:** transactional logic and macOS FFI compile/link passed; no real user network settings were mutated. Administrator authorization dialog, runtime apply/readback and quit/startup recovery need manual acceptance on a desktop.
- **Windows:** source and native WinINet adapter are present. The installed Rust toolchain has no Windows target. `rustup target add x86_64-pc-windows-msvc` failed while cleaning its cached downloads (`No such file or directory`). No Windows compilation, executable or runtime success is claimed.
- **Browser-mode runtime:** separate-profile command arguments are tested. No installed browser was launched or its outgoing traffic independently measured.
- **Distribution:** no signing/notarization, production installer or release publication.

## Manual acceptance remaining

1. Start the desktop app, choose a real SSH key, save both setup steps and connect.
2. Confirm the homepage connects only after the configured HTTPS probe succeeds and both measured timings appear.
3. Kill the server connection or disable its network; confirm latency disappears, reconnect state/countdown appears and connectivity recovers automatically.
4. Disable the main switch during authentication and during backoff; confirm no new SSH attempts and no retained local listeners.
5. Snapshot actual proxy settings, enable system mode, check readback and browsing, then stop and verify original settings restored. Repeat application crash/startup recovery and external-setting changes.
6. Launch Chrome/Edge in browser-only mode and verify its traffic/hostname resolution uses the tunnel while system settings remain unchanged.
7. Repeat the real flow on Windows, including denied permissions, app close and installer startup.
