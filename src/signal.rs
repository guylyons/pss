//! Sending signals to processes from the interactive view.

use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// Ask politely; the process may clean up or ignore it.
    Term,
    /// Cannot be caught or ignored.
    Kill,
}

impl Signal {
    pub fn name(self) -> &'static str {
        match self {
            Signal::Term => "TERM",
            Signal::Kill => "KILL",
        }
    }

    fn number(self) -> libc::c_int {
        match self {
            Signal::Term => libc::SIGTERM,
            Signal::Kill => libc::SIGKILL,
        }
    }
}

/// Send `sig` to `pid`; the error is a message ready for the status line.
pub fn send(pid: i32, sig: Signal) -> Result<(), String> {
    // SAFETY: kill(2) has no memory-safety preconditions.
    if unsafe { libc::kill(pid, sig.number()) } == 0 {
        return Ok(());
    }
    let errno = io::Error::last_os_error().raw_os_error().unwrap_or(0);
    Err(describe(pid, errno))
}

fn describe(pid: i32, errno: i32) -> String {
    match errno {
        libc::EPERM => format!("{pid}: not permitted, it belongs to another user (try sudo pss)"),
        libc::ESRCH => format!("{pid}: no such process (already gone?)"),
        e => format!("{pid}: {}", io::Error::from_raw_os_error(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    use std::process::Command;

    fn sleeper() -> std::process::Child {
        Command::new("/bin/sleep").arg("30").spawn().unwrap()
    }

    #[test]
    fn term_and_kill_reach_the_process() {
        for (sig, number) in [(Signal::Term, libc::SIGTERM), (Signal::Kill, libc::SIGKILL)] {
            let mut child = sleeper();
            send(child.id() as i32, sig).unwrap();
            assert_eq!(child.wait().unwrap().signal(), Some(number));
        }
    }

    #[test]
    fn errors_explain_what_to_do() {
        assert_eq!(
            describe(1, libc::EPERM),
            "1: not permitted, it belongs to another user (try sudo pss)"
        );
        assert_eq!(
            describe(5, libc::ESRCH),
            "5: no such process (already gone?)"
        );
    }

    #[test]
    fn names() {
        assert_eq!(Signal::Term.name(), "TERM");
        assert_eq!(Signal::Kill.name(), "KILL");
    }
}
