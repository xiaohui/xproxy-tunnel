use std::{collections::BTreeMap, sync::Mutex};
#[allow(dead_code)]
#[path = "../src/system/mod.rs"]
mod system;
use serde_json::{json, Value};
use system::{Backend, Transaction};
struct Fixture {
    state: Mutex<BTreeMap<String, Value>>,
    writes: Mutex<usize>,
    fail_at: usize,
    drop_at: usize,
    journal: std::path::PathBuf,
}
impl Backend for Fixture {
    fn snapshot(&self) -> anyhow::Result<BTreeMap<String, Value>> {
        Ok(self.state.lock().unwrap().clone())
    }
    fn desired(
        &self,
        _: u16,
        original: &BTreeMap<String, Value>,
    ) -> anyhow::Result<BTreeMap<String, Value>> {
        Ok(original
            .keys()
            .map(|k| {
                (
                    k.clone(),
                    if k.starts_with("unowned/") {
                        original[k].clone()
                    } else {
                        json!("owned")
                    },
                )
            })
            .collect())
    }
    fn write(&self, k: &str, v: &Value) -> anyhow::Result<()> {
        assert!(self.journal.exists(), "journal must precede mutation");
        let mut n = self.writes.lock().unwrap();
        *n += 1;
        if *n == self.fail_at {
            anyhow::bail!("injected failure")
        }
        if *n != self.drop_at {
            self.state.lock().unwrap().insert(k.into(), v.clone());
        }
        Ok(())
    }
}
fn fixture(dir: &std::path::Path, fail_at: usize) -> Fixture {
    Fixture {
        state: Mutex::new(BTreeMap::from([
            ("a".into(), json!("original")),
            ("b".into(), json!("original")),
        ])),
        writes: Mutex::new(0),
        fail_at,
        drop_at: 0,
        journal: dir.join("system-proxy.json"),
    }
}
#[test]
fn journal_precedes_mutation_and_crash_recovers() {
    let d = tempfile::tempdir().unwrap();
    let b = fixture(d.path(), 0);
    Transaction::new(d.path().into(), &b).apply(1234).unwrap();
    assert_eq!(b.state.lock().unwrap()["a"], json!("owned"));
    Transaction::new(d.path().into(), &b).restore().unwrap();
    assert_eq!(b.state.lock().unwrap()["a"], json!("original"));
    assert!(!b.journal.exists());
}
#[test]
fn partial_failure_rolls_back() {
    let d = tempfile::tempdir().unwrap();
    let b = fixture(d.path(), 2);
    assert!(Transaction::new(d.path().into(), &b).apply(1234).is_err());
    assert_eq!(b.state.lock().unwrap()["a"], json!("original"));
    assert!(!b.journal.exists());
}
#[test]
fn external_changes_are_preserved() {
    let d = tempfile::tempdir().unwrap();
    let b = fixture(d.path(), 0);
    let t = Transaction::new(d.path().into(), &b);
    t.apply(1234).unwrap();
    b.state
        .lock()
        .unwrap()
        .insert("a".into(), json!("external"));
    assert!(t.restore().is_err());
    assert_eq!(b.state.lock().unwrap()["a"], json!("external"));
    assert_eq!(b.state.lock().unwrap()["b"], json!("original"));
    assert!(!b.journal.exists());
}
#[test]
fn corrupted_journal_blocks_apply() {
    let d = tempfile::tempdir().unwrap();
    let b = fixture(d.path(), 0);
    std::fs::write(&b.journal, b"broken").unwrap();
    assert!(Transaction::new(d.path().into(), &b).apply(1234).is_err());
    assert_eq!(*b.writes.lock().unwrap(), 0);
}
#[test]
fn recovery_failure_retains_journal_for_retry() {
    let d = tempfile::tempdir().unwrap();
    let b = fixture(d.path(), 3);
    let t = Transaction::new(d.path().into(), &b);
    t.apply(1234).unwrap();
    assert!(t.restore().is_err());
    assert!(b.journal.exists());
    t.restore().unwrap();
    assert!(!b.journal.exists());
    assert_eq!(b.state.lock().unwrap()["a"], json!("original"));
}

#[test]
fn silent_write_drop_fails_and_rolls_back() {
    let d = tempfile::tempdir().unwrap();
    let mut b = fixture(d.path(), 0);
    b.drop_at = 2;
    assert!(Transaction::new(d.path().into(), &b).apply(1234).is_err());
    assert_eq!(b.state.lock().unwrap()["a"], json!("original"));
    assert!(!b.journal.exists());
}
#[test]
fn active_checks_owned_settings_after_external_change() {
    let d = tempfile::tempdir().unwrap();
    let b = fixture(d.path(), 0);
    let t = Transaction::new(d.path().into(), &b);
    assert!(!t.active().unwrap());
    t.apply(1234).unwrap();
    assert!(t.active().unwrap());
    b.state
        .lock()
        .unwrap()
        .insert("a".into(), json!("external"));
    assert!(!t.active().unwrap());
}
#[test]
fn silent_restore_drop_retains_recovery_journal() {
    let d = tempfile::tempdir().unwrap();
    let mut b = fixture(d.path(), 0);
    b.drop_at = 3;
    let t = Transaction::new(d.path().into(), &b);
    t.apply(1234).unwrap();
    assert!(t.restore().is_err());
    assert!(b.journal.exists());
    t.restore().unwrap();
    assert!(!b.journal.exists());
}

#[cfg(target_os = "macos")]
#[test]
fn macos_targets_enabled_services_only() {
    let original = BTreeMap::from([
        ("enabled/ServiceEnabled".into(), json!(true)),
        ("enabled/HTTPProxy".into(), json!("old")),
        ("disabled/ServiceEnabled".into(), json!(false)),
        ("disabled/HTTPProxy".into(), json!("disabled-old")),
    ]);
    let desired = system::macos::MacOS::default()
        .desired(1234, &original)
        .unwrap();
    assert_eq!(desired["enabled/HTTPProxy"], json!("127.0.0.1"));
    assert_eq!(desired["disabled/HTTPProxy"], json!("disabled-old"));
    assert_eq!(desired["enabled/ServiceEnabled"], json!(true));
}
#[cfg(target_os = "macos")]
#[test]
fn macos_restores_proxy_after_service_disabled() {
    struct ServiceFixture(Fixture);
    impl Backend for ServiceFixture {
        fn snapshot(&self) -> anyhow::Result<BTreeMap<String, Value>> {
            self.0.snapshot()
        }
        fn desired(
            &self,
            p: u16,
            o: &BTreeMap<String, Value>,
        ) -> anyhow::Result<BTreeMap<String, Value>> {
            system::macos::MacOS::default().desired(p, o)
        }
        fn write(&self, k: &str, v: &Value) -> anyhow::Result<()> {
            self.0.write(k, v)
        }
    }
    let d = tempfile::tempdir().unwrap();
    let b = ServiceFixture(fixture(d.path(), 0));
    *b.0.state.lock().unwrap() = BTreeMap::from([
        ("service/ServiceEnabled".into(), json!(true)),
        ("service/HTTPProxy".into(), json!("old")),
    ]);
    let t = Transaction::new(d.path().into(), &b);
    t.apply(1234).unwrap();
    assert_eq!(
        b.0.state.lock().unwrap()["service/ServiceEnabled"],
        json!(true)
    );
    b.0.state
        .lock()
        .unwrap()
        .insert("service/ServiceEnabled".into(), json!(false));
    assert!(t.active().unwrap());
    t.restore().unwrap(); // service enable flag is preserved and never owned
    assert_eq!(b.0.state.lock().unwrap()["service/HTTPProxy"], json!("old"));
    assert_eq!(
        b.0.state.lock().unwrap()["service/ServiceEnabled"],
        json!(false)
    );
    assert!(!b.0.journal.exists());
}

#[test]
fn unrelated_property_change_keeps_active_and_restores_without_warning() {
    let d = tempfile::tempdir().unwrap();
    let b = fixture(d.path(), 0);
    b.state
        .lock()
        .unwrap()
        .insert("unowned/disabled-proxy".into(), json!("before"));
    let t = Transaction::new(d.path().into(), &b);
    t.apply(1234).unwrap();
    b.state
        .lock()
        .unwrap()
        .insert("unowned/disabled-proxy".into(), json!("external"));
    assert!(t.active().unwrap());
    t.restore().unwrap();
    assert_eq!(b.state.lock().unwrap()["a"], json!("original"));
    assert_eq!(
        b.state.lock().unwrap()["unowned/disabled-proxy"],
        json!("external")
    );
    assert!(!b.journal.exists());
}
