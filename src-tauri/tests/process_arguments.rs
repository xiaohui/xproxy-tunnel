use xproxy::{browser::browser_args, model::Config, ssh::ssh_args};
#[test]
fn ssh_is_batch_loopback_host_checked_and_not_shell_interpolated() {
    let c = Config {
        host: "example.com".into(),
        username: "user".into(),
        key_path: "/private/tmp/key with spaces".into(),
        ..Config::default()
    };
    let args = ssh_args(&c, std::path::Path::new("/private/tmp/known hosts"));
    let args = args
        .iter()
        .map(|s| s.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(args.contains(&"127.0.0.1:1080".into()));
    assert!(args.contains(&"BatchMode=yes".into()));
    assert!(args.contains(&"StrictHostKeyChecking=accept-new".into()));
    assert!(args.contains(&"ExitOnForwardFailure=yes".into()));
    assert!(args.contains(&"/private/tmp/key with spaces".into()));
    assert_eq!(args.last().unwrap(), "example.com");
}
#[test]
fn browser_profile_is_independent_and_dns_goes_through_socks() {
    let args = browser_args(std::path::Path::new("/private/tmp/separate profile"), 1080);
    let args = args
        .iter()
        .map(|s| s.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(args.contains(&"--proxy-server=socks5://127.0.0.1:1080".into()));
    assert!(args.contains(&"--user-data-dir=/private/tmp/separate profile".into()));
    assert!(args
        .iter()
        .any(|s| s.starts_with("--host-resolver-rules=MAP * ~NOTFOUND")));
    assert!(args.contains(&"--disable-quic".into()));
}
