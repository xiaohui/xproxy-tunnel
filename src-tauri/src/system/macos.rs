//! SystemConfiguration preserves absent properties and authentication metadata.
use super::Backend;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    ffi::{c_void, CString},
    ptr,
    sync::Mutex,
};
type Ref = *const c_void;
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(v: Ref);
    fn CFStringCreateWithCString(a: Ref, s: *const i8, e: u32) -> Ref;
    fn CFStringGetCString(s: Ref, b: *mut i8, n: isize, e: u32) -> bool;
    fn CFArrayGetCount(a: Ref) -> isize;
    fn CFArrayGetValueAtIndex(a: Ref, i: isize) -> Ref;
    fn CFDictionaryGetValue(d: Ref, k: Ref) -> Ref;
    fn CFDictionaryCreateMutableCopy(a: Ref, n: isize, d: Ref) -> Ref;
    fn CFDictionarySetValue(d: Ref, k: Ref, v: Ref);
    fn CFDictionaryRemoveValue(d: Ref, k: Ref);
    fn CFGetTypeID(v: Ref) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFNumberGetTypeID() -> usize;
    fn CFNumberGetValue(v: Ref, t: i32, n: *mut c_void) -> bool;
    fn CFNumberCreate(a: Ref, t: i32, n: *const c_void) -> Ref;
}
#[link(name = "SystemConfiguration", kind = "framework")]
unsafe extern "C" {
    fn SCPreferencesCreate(a: Ref, name: Ref, id: Ref) -> Ref;
    fn SCPreferencesCreateWithAuthorization(
        a: Ref,
        name: Ref,
        id: Ref,
        authorization: *mut c_void,
    ) -> Ref;
    fn SCPreferencesLock(p: Ref, wait: bool) -> bool;
    fn SCPreferencesUnlock(p: Ref) -> bool;
    fn SCPreferencesCommitChanges(p: Ref) -> bool;
    fn SCPreferencesApplyChanges(p: Ref) -> bool;
    fn SCNetworkServiceCopyAll(p: Ref) -> Ref;
    fn SCNetworkServiceGetEnabled(s: Ref) -> bool;
    fn SCNetworkServiceGetServiceID(s: Ref) -> Ref;
    fn SCNetworkServiceCopyProtocol(s: Ref, t: Ref) -> Ref;
    fn SCNetworkProtocolGetConfiguration(p: Ref) -> Ref;
    fn SCNetworkProtocolSetConfiguration(p: Ref, d: Ref) -> bool;
}
struct Owned(Ref);
impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) }
        }
    }
}
fn string(s: &str) -> Owned {
    let c = CString::new(s).unwrap();
    Owned(unsafe { CFStringCreateWithCString(ptr::null(), c.as_ptr(), 0x08000100) })
}
fn text(v: Ref) -> Result<String> {
    let mut b = vec![0i8; 65536];
    if !unsafe { CFStringGetCString(v, b.as_mut_ptr(), b.len() as isize, 0x08000100) } {
        bail!("SystemConfiguration string conversion failed")
    }
    Ok(unsafe { std::ffi::CStr::from_ptr(b.as_ptr()) }
        .to_string_lossy()
        .into_owned())
}
const PROPERTIES: &[&str] = &[
    "HTTPEnable",
    "HTTPProxy",
    "HTTPPort",
    "HTTPSEnable",
    "HTTPSProxy",
    "HTTPSPort",
    "SOCKSEnable",
    "ProxyAutoConfigEnable",
    "ProxyAutoDiscoveryEnable",
];
fn session() -> Result<Owned> {
    let name = string("xproxy");
    let p = Owned(unsafe { SCPreferencesCreate(ptr::null(), name.0, ptr::null()) });
    if p.0.is_null() {
        bail!("Cannot open system network preferences")
    }
    Ok(p)
}
fn value(v: Ref) -> Result<Value> {
    if v.is_null() {
        return Ok(Value::Null);
    }
    let t = unsafe { CFGetTypeID(v) };
    if t == unsafe { CFStringGetTypeID() } {
        return Ok(json!(text(v)?));
    }
    if t == unsafe { CFNumberGetTypeID() } {
        let mut n = 0i64;
        if !unsafe { CFNumberGetValue(v, 4, &mut n as *mut _ as *mut c_void) } {
            bail!("Invalid proxy number")
        }
        return Ok(json!(n));
    }
    bail!("Unsupported system proxy property type")
}
#[repr(C)]
struct AuthorizationItem {
    name: *const i8,
    value_length: usize,
    value: *mut c_void,
    flags: u32,
}
#[repr(C)]
struct AuthorizationRights {
    count: u32,
    items: *mut AuthorizationItem,
}
#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn AuthorizationCreate(
        rights: *const AuthorizationRights,
        environment: *const AuthorizationRights,
        flags: u32,
        authorization: *mut *mut c_void,
    ) -> i32;
    fn AuthorizationCopyRights(
        authorization: *mut c_void,
        rights: *const AuthorizationRights,
        environment: *const AuthorizationRights,
        flags: u32,
        authorized: *mut *mut AuthorizationRights,
    ) -> i32;
    fn AuthorizationFree(authorization: *mut c_void, flags: u32) -> i32;
}
struct Authorization(*mut c_void);
// The opaque authorization handle is only accessed under MacOS.authorization's mutex.
unsafe impl Send for Authorization {}
impl Drop for Authorization {
    fn drop(&mut self) {
        unsafe {
            AuthorizationFree(self.0, 0);
        }
    }
}
#[derive(Default)]
pub struct MacOS {
    authorization: Mutex<Option<Authorization>>,
}
impl MacOS {
    fn write_session(&self) -> Result<Owned> {
        let mut cached = self
            .authorization
            .lock()
            .map_err(|_| anyhow::anyhow!("系统网络授权状态不可用"))?;
        if cached.is_none() {
            let mut reference = ptr::null_mut();
            let status =
                unsafe { AuthorizationCreate(ptr::null(), ptr::null(), 0, &mut reference) };
            if status != 0 || reference.is_null() {
                bail!("无法创建系统网络授权（{status}）")
            }
            *cached = Some(Authorization(reference));
        }
        let reference = cached.as_ref().unwrap().0;
        // Refresh expired rights; authorization remains in memory and the OS owns the password dialog.
        let name = CString::new("system.services.systemconfiguration.network").unwrap();
        let mut item = AuthorizationItem {
            name: name.as_ptr(),
            value_length: 0,
            value: ptr::null_mut(),
            flags: 0,
        };
        let rights = AuthorizationRights {
            count: 1,
            items: &mut item,
        };
        // interactionAllowed (1) | extendRights (2).
        let status =
            unsafe { AuthorizationCopyRights(reference, &rights, ptr::null(), 3, ptr::null_mut()) };
        if status != 0 {
            *cached = None;
            bail!("系统网络授权未获批准（{status}），请允许管理员授权后重试")
        }
        let name = string("XProxy");
        let p = Owned(unsafe {
            SCPreferencesCreateWithAuthorization(
                ptr::null(),
                name.0,
                ptr::null(),
                cached.as_ref().unwrap().0,
            )
        });
        if p.0.is_null() {
            bail!("无法创建已授权的系统网络配置会话")
        }
        Ok(p)
    }
}
impl Backend for MacOS {
    fn snapshot(&self) -> Result<BTreeMap<String, Value>> {
        let p = session()?;
        let services = Owned(unsafe { SCNetworkServiceCopyAll(p.0) });
        if services.0.is_null() {
            bail!("Cannot enumerate network services")
        }
        let kind = string("Proxies");
        let mut out = BTreeMap::new();
        for i in 0..unsafe { CFArrayGetCount(services.0) } {
            let s = unsafe { CFArrayGetValueAtIndex(services.0, i) };
            let id = text(unsafe { SCNetworkServiceGetServiceID(s) })?;
            let protocol = Owned(unsafe { SCNetworkServiceCopyProtocol(s, kind.0) });
            if protocol.0.is_null() {
                continue;
            }
            let dict = unsafe { SCNetworkProtocolGetConfiguration(protocol.0) };
            if dict.is_null() {
                bail!("Network service has no proxy configuration")
            }
            // Enumerate disabled services too: they may still hold settings owned by a recovery journal.
            out.insert(
                format!("{id}/ServiceEnabled"),
                json!(unsafe { SCNetworkServiceGetEnabled(s) }),
            );
            for key in PROPERTIES {
                let k = string(key);
                out.insert(
                    format!("{id}/{key}"),
                    value(unsafe { CFDictionaryGetValue(dict, k.0) })?,
                );
            }
        }
        if out.is_empty() {
            bail!("No network services support proxies")
        }
        Ok(out)
    }
    fn desired(
        &self,
        port: u16,
        original: &BTreeMap<String, Value>,
    ) -> Result<BTreeMap<String, Value>> {
        if !original
            .iter()
            .any(|(key, v)| key.ends_with("/ServiceEnabled") && v.as_bool() == Some(true))
        {
            bail!("没有已启用且支持代理的网络服务")
        }
        original
            .iter()
            .map(|(key, original_value)| {
                let (service, property) = key
                    .split_once('/')
                    .ok_or_else(|| anyhow::anyhow!("Invalid proxy key"))?;
                let enabled = original
                    .get(&format!("{service}/ServiceEnabled"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let value = if !enabled || property == "ServiceEnabled" {
                    original_value.clone()
                } else {
                    match property {
                        "HTTPEnable" | "HTTPSEnable" => json!(1),
                        "HTTPProxy" | "HTTPSProxy" => json!("127.0.0.1"),
                        "HTTPPort" | "HTTPSPort" => json!(port),
                        "SOCKSEnable" | "ProxyAutoConfigEnable" | "ProxyAutoDiscoveryEnable" => {
                            json!(0)
                        }
                        _ => bail!("Unknown proxy property"),
                    }
                };
                Ok((key.clone(), value))
            })
            .collect()
    }

    fn write(&self, key: &str, v: &Value) -> Result<()> {
        let (id, property) = key
            .split_once('/')
            .ok_or_else(|| anyhow::anyhow!("Invalid proxy key"))?;
        if !PROPERTIES.contains(&property) {
            bail!("Unknown proxy property")
        }
        let p = self.write_session()?;
        if !unsafe { SCPreferencesLock(p.0, false) } {
            bail!("网络配置正在被修改，或系统授权被拒绝")
        }
        let result = (|| {
            let services = Owned(unsafe { SCNetworkServiceCopyAll(p.0) });
            if services.0.is_null() {
                bail!("Cannot enumerate network services")
            }
            let kind = string("Proxies");
            for i in 0..unsafe { CFArrayGetCount(services.0) } {
                let s = unsafe { CFArrayGetValueAtIndex(services.0, i) };
                if text(unsafe { SCNetworkServiceGetServiceID(s) })? != id {
                    continue;
                }
                let protocol = Owned(unsafe { SCNetworkServiceCopyProtocol(s, kind.0) });
                if protocol.0.is_null() {
                    bail!("Proxy service removed")
                };
                let old = unsafe { SCNetworkProtocolGetConfiguration(protocol.0) };
                if old.is_null() {
                    bail!("Proxy configuration removed")
                }
                let dict = Owned(unsafe { CFDictionaryCreateMutableCopy(ptr::null(), 0, old) });
                let k = string(property);
                if v.is_null() {
                    unsafe { CFDictionaryRemoveValue(dict.0, k.0) }
                } else {
                    let data = if let Some(s) = v.as_str() {
                        string(s)
                    } else if let Some(n) = v.as_i64() {
                        Owned(unsafe {
                            CFNumberCreate(ptr::null(), 4, &n as *const _ as *const c_void)
                        })
                    } else {
                        bail!("Invalid proxy journal value")
                    };
                    unsafe { CFDictionarySetValue(dict.0, k.0, data.0) }
                }
                if !unsafe {
                    SCNetworkProtocolSetConfiguration(protocol.0, dict.0)
                        && SCPreferencesCommitChanges(p.0)
                        && SCPreferencesApplyChanges(p.0)
                } {
                    bail!("系统代理写入失败，请检查系统网络授权")
                };
                return Ok(());
            }
            bail!("Network service removed")
        })();
        unsafe { SCPreferencesUnlock(p.0) };
        result
    }
}
