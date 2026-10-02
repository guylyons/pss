//! What each piece of text *means* ([`Role`]) and how that looks ([`theme`]).
//! Formatting code only tags text with roles; this is the one place colors live.

use crate::process::RunState;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UserKind {
    Root,
    Me,
    /// Another user; the value picks a stable palette color.
    Other(u32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Role {
    Plain,
    Header,
    /// A value the OS would not give us (`-`).
    Missing,
    Pcpu(f64),
    Pmem(f64),
    Rss(u64),
    Vsz,
    State(RunState),
    StateFlags,
    User(UserKind),
    /// Directory part of argv[0].
    CmdDir,
    /// Executable name.
    CmdExe,
    /// An argument starting with `-`.
    CmdFlag,
    CmdArg,
    /// Text matching the search pattern given on the command line.
    Match,
    /// Text matching the pager's `/` search.
    SearchHit,
    /// Whole-row states in the interactive view, drawn over the row's text.
    Selected,
    /// Waiting for the user to confirm killing it.
    Doomed,
    /// Sent a signal; still there until the next refresh shows it gone.
    Dying,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    Black,
    /// xterm 256-color index.
    Indexed(u8),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub dim: bool,
    pub underline: bool,
    pub strike: bool,
}

impl Style {
    const fn fg(c: Color) -> Style {
        Style {
            fg: Some(c),
            bg: None,
            bold: false,
            dim: false,
            underline: false,
            strike: false,
        }
    }
    const DIM: Style = Style {
        fg: None,
        bg: None,
        bold: false,
        dim: true,
        underline: false,
        strike: false,
    };
    const BOLD: Style = Style {
        fg: None,
        bg: None,
        bold: true,
        dim: false,
        underline: false,
        strike: false,
    };
    const fn bold(mut self) -> Style {
        self.bold = true;
        self
    }
}

/// Palette for other users: avoids red (root) and green (you).
const USER_PALETTE: [Color; 6] = [
    Color::Cyan,
    Color::Blue,
    Color::Magenta,
    Color::Indexed(208), // orange
    Color::Indexed(141), // lavender
    Color::Indexed(37),  // teal
];

pub fn theme(role: Role) -> Style {
    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * MIB;
    match role {
        Role::Plain | Role::CmdArg => Style::default(),
        Role::Header => Style {
            bold: true,
            underline: true,
            ..Style::default()
        },
        Role::Missing | Role::Vsz | Role::StateFlags | Role::CmdDir => Style::DIM,
        Role::Pcpu(v) => heat(v, 1.0, 10.0, 50.0),
        Role::Pmem(v) => heat(v, 1.0, 5.0, 20.0),
        Role::Rss(b) => heat(b as f64, (100 * MIB) as f64, GIB as f64, (4 * GIB) as f64),
        Role::State(s) => match s {
            RunState::Running => Style::fg(Color::Green).bold(),
            RunState::Stopped => Style::fg(Color::Yellow),
            RunState::Zombie | RunState::Uninterruptible => Style::fg(Color::Red).bold(),
            RunState::Sleeping | RunState::Idle | RunState::Halted | RunState::Unknown => {
                Style::DIM
            }
        },
        Role::User(UserKind::Root) => Style::fg(Color::Red),
        Role::User(UserKind::Me) => Style::fg(Color::Green).bold(),
        Role::User(UserKind::Other(h)) => Style::fg(USER_PALETTE[h as usize % USER_PALETTE.len()]),
        Role::CmdExe => Style::BOLD,
        Role::CmdFlag => Style::fg(Color::Cyan),
        Role::Match => Style {
            fg: Some(Color::Black),
            bg: Some(Color::Yellow),
            bold: true,
            ..Style::default()
        },
        Role::SearchHit => Style {
            fg: Some(Color::Black),
            bg: Some(Color::Cyan),
            bold: true,
            ..Style::default()
        },
        Role::Selected => Style {
            bg: Some(Color::Indexed(237)), // dark gray: keeps the cells' own colors readable
            ..Style::default()
        },
        Role::Doomed => Style {
            fg: Some(Color::Indexed(231)), // white
            bg: Some(Color::Indexed(124)), // deep red
            bold: true,
            ..Style::default()
        },
        Role::Dying => Style {
            fg: Some(Color::Indexed(174)), // faded red
            strike: true,
            ..Style::default()
        },
    }
}

/// Below `low` dim, below `mid` normal, below `high` yellow, else bold red.
fn heat(v: f64, low: f64, mid: f64, high: f64) -> Style {
    if v < low {
        Style::DIM
    } else if v < mid {
        Style::default()
    } else if v < high {
        Style::fg(Color::Yellow)
    } else {
        Style::fg(Color::Red).bold()
    }
}

/// A stable small hash so a user keeps the same color across runs.
pub fn user_hash(name: &str) -> u32 {
    name.bytes()
        .fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_heat_thresholds() {
        assert!(theme(Role::Pcpu(0.2)).dim);
        assert_eq!(theme(Role::Pcpu(5.0)), Style::default());
        assert_eq!(theme(Role::Pcpu(20.0)).fg, Some(Color::Yellow));
        assert_eq!(theme(Role::Pcpu(90.0)).fg, Some(Color::Red));
    }

    #[test]
    fn users_get_distinct_stable_colors() {
        assert_eq!(theme(Role::User(UserKind::Root)).fg, Some(Color::Red));
        assert_eq!(theme(Role::User(UserKind::Me)).fg, Some(Color::Green));
        let a = theme(Role::User(UserKind::Other(user_hash("_windowserver"))));
        assert_eq!(
            a,
            theme(Role::User(UserKind::Other(user_hash("_windowserver"))))
        );
        assert!(!matches!(a.fg, Some(Color::Red | Color::Green)));
    }

    #[test]
    fn zombie_is_loud_sleeping_is_quiet() {
        assert_eq!(theme(Role::State(RunState::Zombie)).fg, Some(Color::Red));
        assert!(theme(Role::State(RunState::Sleeping)).dim);
    }
}
