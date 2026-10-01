//! uid → user name lookups, cached because a snapshot asks for the same
//! few uids hundreds of times.

use std::collections::HashMap;
use std::ffi::CStr;

#[derive(Default)]
pub struct UserCache {
    names: HashMap<u32, String>,
}

impl UserCache {
    pub fn name(&mut self, uid: u32) -> String {
        self.names
            .entry(uid)
            .or_insert_with(|| lookup(uid).unwrap_or_else(|| uid.to_string()))
            .clone()
    }
}

fn lookup(uid: u32) -> Option<String> {
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    let mut buf = vec![0 as libc::c_char; 4096];
    // SAFETY: every pointer refers to a live local of the right size.
    let rc = unsafe { libc::getpwuid_r(uid, &mut pw, buf.as_mut_ptr(), buf.len(), &mut result) };
    if rc != 0 || result.is_null() || pw.pw_name.is_null() {
        return None;
    }
    // SAFETY: getpwuid_r succeeded, so pw_name points into `buf`.
    Some(
        unsafe { CStr::from_ptr(pw.pw_name) }
            .to_string_lossy()
            .into_owned(),
    )
}

/// Resolve a user name to a uid, for `-u name` / `-U name`.
pub fn uid_for_name(name: &str) -> Option<u32> {
    let cname = std::ffi::CString::new(name).ok()?;
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    let mut buf = vec![0 as libc::c_char; 4096];
    // SAFETY: as in `lookup`.
    let rc = unsafe {
        libc::getpwnam_r(
            cname.as_ptr(),
            &mut pw,
            buf.as_mut_ptr(),
            buf.len(),
            &mut result,
        )
    };
    (rc == 0 && !result.is_null()).then_some(pw.pw_uid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_is_uid_zero() {
        assert_eq!(UserCache::default().name(0), "root");
        assert_eq!(uid_for_name("root"), Some(0));
    }

    #[test]
    fn unknown_uid_falls_back_to_number() {
        assert_eq!(UserCache::default().name(4_000_000_000), "4000000000");
        assert_eq!(uid_for_name("no-such-user-pss"), None);
    }
}
