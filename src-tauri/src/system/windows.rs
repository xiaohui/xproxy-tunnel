//! Current-user default LAN WinINet settings (not machine-wide WinHTTP).
use super::Backend;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::{collections::BTreeMap, ffi::c_void, ptr};
#[repr(C)]
union Data {
    number: u32,
    text: *mut u16,
    time: [u32; 2],
}
#[repr(C)]
struct OptionW {
    option: u32,
    value: Data,
}
#[repr(C)]
struct ListW {
    size: u32,
    connection: *mut u16,
    count: u32,
    error: u32,
    options: *mut OptionW,
}
#[link(name = "wininet")]
unsafe extern "system" {
    fn InternetQueryOptionW(h: *mut c_void, option: u32, data: *mut c_void, len: *mut u32) -> i32;
    fn InternetSetOptionW(h: *mut c_void, option: u32, data: *mut c_void, len: u32) -> i32;
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GlobalFree(h: *mut c_void) -> *mut c_void;
}
const PER_CONNECTION: u32 = 75;
const KEYS: &[(&str, u32)] = &[("flags", 1), ("proxy", 2), ("bypass", 3), ("pac", 4)];
fn list(options: &mut [OptionW]) -> ListW {
    ListW {
        size: std::mem::size_of::<ListW>() as u32,
        connection: ptr::null_mut(),
        count: options.len() as u32,
        error: 0,
        options: options.as_mut_ptr(),
    }
}
fn checked(ok: i32) -> Result<()> {
    if ok == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
unsafe fn decode(p: *mut u16) -> Result<Value> {
    if p.is_null() {
        return Ok(Value::Null);
    }
    let mut n = 0;
    while n < 1024 * 1024 && unsafe { *p.add(n) } != 0 {
        n += 1
    }
    if n == 1024 * 1024 {
        bail!("Invalid WinINet string length")
    }
    let s = String::from_utf16(unsafe { std::slice::from_raw_parts(p, n) })?;
    Ok(json!(s))
}
pub struct Windows;
impl Backend for Windows {
    fn snapshot(&self) -> Result<BTreeMap<String, Value>> {
        let mut options: Vec<_> = KEYS
            .iter()
            .map(|(_, id)| OptionW {
                option: *id,
                value: Data { number: 0 },
            })
            .collect();
        let mut l = list(&mut options);
        let mut len = l.size;
        checked(unsafe {
            InternetQueryOptionW(
                ptr::null_mut(),
                PER_CONNECTION,
                &mut l as *mut _ as *mut c_void,
                &mut len,
            )
        })?;
        let result = (|| {
            let mut out = BTreeMap::new();
            for ((k, _), o) in KEYS.iter().zip(&options) {
                out.insert(
                    (*k).into(),
                    if *k == "flags" {
                        json!(unsafe { o.value.number })
                    } else {
                        unsafe { decode(o.value.text) }?
                    },
                );
            }
            Ok(out)
        })();
        for (i, o) in options.iter().enumerate() {
            if i > 0 {
                unsafe { GlobalFree(o.value.text as *mut c_void) };
            }
        }
        result
    }
    fn desired(
        &self,
        port: u16,
        original: &BTreeMap<String, Value>,
    ) -> Result<BTreeMap<String, Value>> {
        let mut out = original.clone();
        out.insert("flags".into(), json!(3));
        out.insert(
            "proxy".into(),
            json!(format!("http=127.0.0.1:{port};https=127.0.0.1:{port}")),
        );
        out.insert("bypass".into(), json!("<local>")); /* PAC URL retained, but auto PAC and discovery flags disabled. */
        Ok(out)
    }
    fn write(&self, key: &str, value: &Value) -> Result<()> {
        let id = KEYS
            .iter()
            .find(|(k, _)| *k == key)
            .ok_or_else(|| anyhow::anyhow!("Invalid WinINet property"))?
            .1;
        let mut wide = Vec::new();
        let data = if key == "flags" {
            let n = value
                .as_u64()
                .filter(|n| *n <= u32::MAX as u64)
                .ok_or_else(|| anyhow::anyhow!("Invalid WinINet flags"))?;
            Data { number: n as u32 }
        } else if value.is_null() {
            Data {
                text: ptr::null_mut(),
            }
        } else {
            let s = value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Invalid WinINet string"))?;
            if s.contains('\0') {
                bail!("Invalid WinINet embedded NUL")
            };
            wide.extend(s.encode_utf16());
            wide.push(0);
            Data {
                text: wide.as_mut_ptr(),
            }
        };
        let mut options = [OptionW {
            option: id,
            value: data,
        }];
        let mut l = list(&mut options);
        checked(unsafe {
            InternetSetOptionW(
                ptr::null_mut(),
                PER_CONNECTION,
                &mut l as *mut _ as *mut c_void,
                l.size,
            )
        })?;
        checked(unsafe { InternetSetOptionW(ptr::null_mut(), 39, ptr::null_mut(), 0) })?;
        checked(unsafe { InternetSetOptionW(ptr::null_mut(), 37, ptr::null_mut(), 0) })?;
        Ok(())
    }
}
