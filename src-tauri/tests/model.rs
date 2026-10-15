use xproxy::model::{retry_seconds, Config, Phase, Status};

#[test]
fn reject_option_injection_and_nonexistent_key() {
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("key");
    std::fs::write(&key, "fixture").unwrap();
    let mut config = Config {
        host: "203.0.113.10".into(),
        username: "ubuntu".into(),
        key_path: key.to_string_lossy().into(),
        ..Config::default()
    };
    config.validate().unwrap();
    config.host = "-oProxyCommand=evil".into();
    assert!(config.validate().is_err());
    config.host = "example.com".into();
    config.username = "user\nother".into();
    assert!(config.validate().is_err());
    config.username = "ubuntu".into();
    config.key_path = dir.path().join("missing").to_string_lossy().into();
    assert!(config.validate().is_err());
}

#[test]
fn reject_colliding_ports_and_non_https_probe() {
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("key");
    std::fs::write(&key, "fixture").unwrap();
    let mut c = Config {
        host: "::1".into(),
        username: "user".into(),
        key_path: key.to_string_lossy().into(),
        ..Config::default()
    };
    c.validate().unwrap();
    c.http_port = c.socks_port;
    assert!(c.validate().is_err());
    c.http_port = 1081;
    c.probe_url = "http://example.com".into();
    assert!(c.validate().is_err());
}

#[test]
fn retry_backoff_is_bounded() {
    assert_eq!(
        (1..=8).map(retry_seconds).collect::<Vec<_>>(),
        vec![1, 2, 4, 8, 16, 30, 30, 30]
    );
    assert_eq!(retry_seconds(u32::MAX), 30);
}

#[test]
fn failed_connection_never_retains_live_metrics() {
    let mut s = Status {
        desired_on: true,
        phase: Phase::Connected,
        latency_ms: Some(86),
        connect_ms: Some(1200),
        connected_at_ms: Some(1),
        ..Status::default()
    };
    s.failed("probe failed".into());
    assert!(matches!(s.phase, Phase::Reconnecting));
    assert!(s.latency_ms.is_none());
    assert!(s.connect_ms.is_none());
    assert!(s.connected_at_ms.is_none());
    assert!(s.desired_on);
}
