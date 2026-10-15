# XProxy SSH desktop v0.1

Goal: deliver the selected status-first homepage and guided settings with a real SSH SOCKS5 tunnel on macOS and Windows.

Architecture: Tauri 2 / React UI invokes a Rust supervisor. OpenSSH handles SSH authentication, host keys and dynamic forwarding. A Rust HTTP/CONNECT bridge forwards through SOCKS5 for system proxy compatibility. Native platform adapters snapshot and restore system settings using an on-disk recovery journal. Browser mode launches Chrome/Edge with a separate profile.

## Approved behavior

- Homepage uses design/home-reference.png; settings uses design/settings-reference.png. No simulated connected state in the desktop runtime.
- Connection desired state differs from observed state. Enable creates one supervisor, disable cancels retries, closes active forwarding and restores system proxy. Configuration is immutable while running.
- Validate IP/host, username, file existence, loopback-only ports, and HTTPS probe URL before starting. Keys are referenced by path. Encrypted keys use an already-unlocked ssh-agent; no passwords are persisted.
- OpenSSH uses strict host checking with first-use trust, dedicated known_hosts, batch authentication, bounded handshake and keepalives. Changed host keys cause an explicit failure.
- Full HTTPS health probe through SOCKS5 establishes connected state; repeat every 10 seconds. Process exit or probe failure removes live latency and triggers retries (1/2/4/8/16/30 seconds). Manual off cancels any pending attempt.
- System mode uses local HTTP/HTTPS proxy; browser-only mode leaves system settings untouched. macOS applies to enabled network services; Windows applies to the current user's default WinINet connection, not WinHTTP or arbitrary apps.
- Capture original settings before mutation; recover on clean shutdown and on next startup. Do not replace settings that another actor changed while running.
- Browser mode launches a separate Chrome/Edge profile, SOCKS5 and remote DNS. No browser embedded into Tauri is advertised.

## Execution and verification

- [x] Rust model tests: configuration, retry timing, inactive latency, manual-off semantics. First run failing tests, then implement.
- [x] Transport tests: SOCKS5 success/failure, HTTP/CONNECT forwarding, listener cancellation and in-flight connections. Local fixtures only.
- [x] System tests: snapshot before mutate, partial failure rollback, conditional recovery, corrupted recovery record.
- [x] Supervisor tests: process exit, repeated health failures, disconnect during connect/backoff, no duplicate tasks. Injected fixtures; do not alter user's networking during automated checks.
- [x] Implement React homepage, SSH step, scope step, file selection, live status, errors, retry countdown, metrics and browser launch.
- [x] Tauri integration: commands, native file picker, single-instance locking, clean exit. Native bundle compiled; GUI interaction still needs visual acceptance.
- [ ] Run Rust tests/clippy/format, frontend build and interaction checks. Compare browser-rendered captures to both visual references and save design-qa.md.
- [x] Document exact launch/build commands, Windows prerequisites and unverified runtime boundaries. No remote credentials were provided; remote-server and actual system-settings acceptance must be reported separately.

This checkout has no Git metadata. All work remains local; there is no commit or PR step.

Final checks: Rust core 29, frontend 8, starter packaging 4, guardian subprocess 2; all passed. Clippy all targets and format passed. macOS debug .app bundled. Remaining runtime/visual acceptance is documented in verification.md; design-qa.md remains blocked by saved/managed browser origin policy despite user authorization.
