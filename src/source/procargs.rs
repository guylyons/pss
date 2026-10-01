//! Parsing the buffer returned by `sysctl(KERN_PROCARGS2)`.
//!
//! Layout: a native-endian `i32` argc, the executable path, NUL padding,
//! then `argc` NUL-terminated argv strings, then the environment.

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn parse_procargs2(buf: &[u8]) -> Option<Vec<String>> {
    let argc_bytes: [u8; 4] = buf.get(..4)?.try_into().ok()?;
    let argc = i32::from_ne_bytes(argc_bytes);
    if argc <= 0 {
        return None;
    }
    let mut rest = &buf[4..];

    // Skip the executable path, then the NUL padding after it.
    let path_end = rest.iter().position(|&b| b == 0)?;
    rest = &rest[path_end..];
    let first_arg = rest.iter().position(|&b| b != 0)?;
    rest = &rest[first_arg..];

    let mut args = Vec::with_capacity(argc as usize);
    for part in rest.split(|&b| b == 0).take(argc as usize) {
        args.push(String::from_utf8_lossy(part).into_owned());
    }
    (args.len() == argc as usize).then_some(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(argc: i32, body: &[u8]) -> Vec<u8> {
        let mut v = argc.to_ne_bytes().to_vec();
        v.extend_from_slice(body);
        v
    }

    #[test]
    fn parses_args_and_ignores_env() {
        let b = buf(2, b"/bin/ls\0\0\0\0ls\0-la\0HOME=/Users/x\0\0");
        assert_eq!(parse_procargs2(&b), Some(vec!["ls".into(), "-la".into()]));
    }

    #[test]
    fn keeps_spaces_inside_an_argument() {
        let b = buf(1, b"/A B/c\0\0/A B/c\0");
        assert_eq!(parse_procargs2(&b), Some(vec!["/A B/c".into()]));
    }

    #[test]
    fn rejects_truncated_and_empty_buffers() {
        assert_eq!(parse_procargs2(&[1, 0]), None);
        assert_eq!(parse_procargs2(&buf(0, b"/bin/x\0\0")), None);
        assert_eq!(parse_procargs2(&buf(3, b"/bin/x\0\0x\0")), None);
    }
}
