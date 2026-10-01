//! Turning raw values into the strings `ps` prints.

use std::ffi::CString;
use std::time::Duration;

/// Accumulated CPU time as `m:ss.hh` (minutes are unbounded), like macOS ps.
pub fn cpu_time(d: Duration) -> String {
    let centis = d.as_millis() / 10;
    let (mins, rem) = (centis / 6000, centis % 6000);
    format!("{}:{:02}.{:02}", mins, rem / 100, rem % 100)
}

/// Elapsed time as `[[dd-]hh:]mm:ss`.
pub fn elapsed(secs: i64) -> String {
    let secs = secs.max(0);
    let (days, hours, mins, s) = (secs / 86400, secs / 3600 % 24, secs / 60 % 60, secs % 60);
    match (days, hours) {
        (0, 0) => format!("{mins:02}:{s:02}"),
        (0, h) => format!("{h:02}:{mins:02}:{s:02}"),
        (d, h) => format!("{d:02}-{h:02}:{mins:02}:{s:02}"),
    }
}

/// Bytes as whole KiB, the unit `ps` uses for VSZ and RSS.
pub fn kib(bytes: u64) -> String {
    (bytes / 1024).to_string()
}

/// Bytes as a short human size: `980K`, `12M`, `1.2G`.
pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes}B")
    } else if v < 10.0 {
        format!("{v:.1}{}", UNITS[unit])
    } else {
        format!("{:.0}{}", v, UNITS[unit])
    }
}

/// The TT column's two-to-four letter form: `ttys000` → `s000`.
pub fn tt(tty: Option<&str>) -> String {
    match tty {
        None => "??".into(),
        Some(t) => t.strip_prefix("tty").unwrap_or(t).into(),
    }
}

/// The strftime format the STARTED column uses for a given age, as BSD ps does.
pub fn start_format(start: i64, now: i64) -> &'static str {
    let age = now - start;
    if age < 24 * 3600 {
        "%-l:%M%p"
    } else if age < 7 * 24 * 3600 {
        "%a%I%p"
    } else {
        "%e%b%y"
    }
}

pub fn start(start: i64, now: i64) -> String {
    strftime_local(start, start_format(start, now))
        .trim()
        .to_string()
}

pub fn lstart(start: i64) -> String {
    strftime_local(start, "%a %b %e %T %Y")
}

fn strftime_local(t: i64, fmt: &str) -> String {
    let Ok(cfmt) = CString::new(fmt) else {
        return String::new();
    };
    // SAFETY: localtime_r and strftime write only into the provided locals.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        let tt: libc::time_t = t;
        if libc::localtime_r(&tt, &mut tm).is_null() {
            return String::new();
        }
        let mut buf = [0u8; 64];
        let n = libc::strftime(
            buf.as_mut_ptr() as *mut libc::c_char,
            buf.len(),
            cfmt.as_ptr(),
            &tm,
        );
        String::from_utf8_lossy(&buf[..n]).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_time_matches_ps() {
        assert_eq!(cpu_time(Duration::from_millis(140)), "0:00.14");
        assert_eq!(
            cpu_time(Duration::from_secs(244 * 60 + 52) + Duration::from_millis(300)),
            "244:52.30"
        );
    }

    #[test]
    fn elapsed_grows_fields() {
        assert_eq!(elapsed(5), "00:05");
        assert_eq!(elapsed(3 * 3600 + 61), "03:01:01");
        assert_eq!(elapsed(2 * 86400 + 7), "02-00:00:07");
    }

    #[test]
    fn sizes() {
        assert_eq!(kib(236048 * 1024), "236048");
        assert_eq!(human(512), "512B");
        assert_eq!(human(980 * 1024), "980K");
        assert_eq!(human(12 * 1024 * 1024), "12M");
        assert_eq!(human(1288490189), "1.2G");
    }

    #[test]
    fn tt_abbreviates() {
        assert_eq!(tt(Some("ttys000")), "s000");
        assert_eq!(tt(Some("console")), "console");
        assert_eq!(tt(None), "??");
    }

    #[test]
    fn start_format_by_age() {
        assert_eq!(start_format(0, 3600), "%-l:%M%p");
        assert_eq!(start_format(0, 3 * 86400), "%a%I%p");
        assert_eq!(start_format(0, 30 * 86400), "%e%b%y");
    }
}
