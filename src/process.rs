//! Platform-neutral process snapshot types.

use std::time::Duration;

/// The scheduler state of a process, reduced to the letters `ps` prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RunState {
    Running,
    Uninterruptible,
    Sleeping,
    Idle,
    Stopped,
    Halted,
    Zombie,
    /// The OS would not tell us (usually another user's process without root).
    #[default]
    Unknown,
}

impl RunState {
    pub fn letter(self) -> char {
        match self {
            RunState::Running => 'R',
            RunState::Uninterruptible => 'U',
            RunState::Sleeping => 'S',
            RunState::Idle => 'I',
            RunState::Stopped => 'T',
            RunState::Halted => 'H',
            RunState::Zombie => 'Z',
            RunState::Unknown => '?',
        }
    }
}

/// The STAT modifier characters that follow the state letter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StateFlags {
    pub foreground: bool,     // +
    pub high_priority: bool,  // <
    pub low_priority: bool,   // N
    pub session_leader: bool, // s
    pub traced: bool,         // X
    pub exiting: bool,        // E
    pub vfork_wait: bool,     // V
}

impl StateFlags {
    pub fn suffix(self) -> String {
        let mut s = String::new();
        for (on, c) in [
            (self.foreground, '+'),
            (self.high_priority, '<'),
            (self.low_priority, 'N'),
            (self.session_leader, 's'),
            (self.vfork_wait, 'V'),
            (self.exiting, 'E'),
            (self.traced, 'X'),
        ] {
            if on {
                s.push(c);
            }
        }
        s
    }
}

/// One process. Fields the OS refuses to report (typically other users'
/// processes when not running as root) are `None`.
#[derive(Debug, Clone, Default)]
pub struct Process {
    pub pid: i32,
    pub ppid: i32,
    pub pgid: i32,
    pub uid: u32,
    pub ruid: u32,
    pub user: String,
    pub ruser: String,
    /// Short command name from the kernel (at most 16 bytes on macOS).
    pub comm: String,
    /// Full argv, if readable.
    pub args: Option<Vec<String>>,
    /// Controlling terminal device name without `/dev/`, e.g. `ttys000`.
    pub tty: Option<String>,
    pub state: RunState,
    pub flags: StateFlags,
    pub nice: i32,
    pub priority: i32,
    pub raw_flags: u32,
    /// Start time, seconds since the Unix epoch.
    pub start: i64,
    pub cpu_time: Option<Duration>,
    pub pcpu: Option<f64>,
    /// Resident and virtual size in bytes.
    pub rss: Option<u64>,
    pub vsz: Option<u64>,
    pub threads: Option<u32>,
}

impl Process {
    /// A short name for messages: the executable's file name, or the
    /// kernel's short name when argv is unreadable.
    pub fn name(&self) -> &str {
        match self.args.as_ref().and_then(|a| a.first()) {
            Some(argv0) => argv0.rsplit('/').next().unwrap_or(argv0),
            None => &self.comm,
        }
    }

    /// The text a COMMAND column shows: argv joined by spaces, or the
    /// kernel's short name in parentheses when argv is unreadable.
    pub fn command_line(&self) -> String {
        match &self.args {
            Some(a) if !a.is_empty() => a.join(" "),
            _ => format!("({})", self.comm),
        }
    }
}

/// Everything collected in one pass.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub processes: Vec<Process>,
    /// Physical memory in bytes, for %MEM.
    pub total_memory: u64,
    /// Seconds since the Unix epoch when the snapshot was taken.
    pub now: i64,
    pub my_uid: u32,
    pub my_pid: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_suffix_order_matches_ps() {
        let f = StateFlags {
            foreground: true,
            session_leader: true,
            low_priority: true,
            ..Default::default()
        };
        assert_eq!(f.suffix(), "+Ns");
    }

    #[test]
    fn name_is_argv0_basename_or_comm() {
        let mut p = Process {
            comm: "2.1.286".into(),
            ..Default::default()
        };
        assert_eq!(p.name(), "2.1.286");
        p.args = Some(vec!["/usr/local/bin/claude".into(), "--resume".into()]);
        assert_eq!(p.name(), "claude");
    }

    #[test]
    fn command_line_falls_back_to_comm() {
        let mut p = Process {
            comm: "launchd".into(),
            ..Default::default()
        };
        assert_eq!(p.command_line(), "(launchd)");
        p.args = Some(vec!["/sbin/launchd".into(), "-v".into()]);
        assert_eq!(p.command_line(), "/sbin/launchd -v");
    }
}
