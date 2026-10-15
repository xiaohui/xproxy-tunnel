use xproxy::shutdown::{ExitDecision, ShutdownGate};
#[test]
fn repeated_quit_cannot_bypass_pending_cleanup() {
    let gate = ShutdownGate::default();
    assert_eq!(gate.request(), ExitDecision::Begin);
    for _ in 0..100 {
        assert_eq!(gate.request(), ExitDecision::Wait);
    }
    gate.cleaned();
    assert_eq!(gate.request(), ExitDecision::Allow);
}
#[test]
fn failed_cleanup_keeps_app_alive_and_can_be_retried() {
    let gate = ShutdownGate::default();
    assert_eq!(gate.request(), ExitDecision::Begin);
    gate.retry();
    assert_eq!(gate.request(), ExitDecision::Begin);
    assert_eq!(gate.request(), ExitDecision::Wait);
}
