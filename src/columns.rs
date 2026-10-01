//! Column keywords (`-o pid,user,...`): headers, cell rendering, sort values.

use std::cmp::Ordering;

use crate::format;
use crate::process::Process;
use crate::style::{Role, UserKind, user_hash};
use crate::text::{Line, Span};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kw {
    Pid,
    Ppid,
    Pgid,
    Uid,
    User,
    Ruid,
    Ruser,
    Pcpu,
    Pmem,
    Vsz,
    Rss,
    Tt,
    Tty,
    Stat,
    Start,
    Lstart,
    Etime,
    Time,
    Nice,
    Pri,
    Comm,
    Ucomm,
    Command,
    Args,
    Flags,
    C,
    Nlwp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

struct Info {
    kw: Kw,
    names: &'static [&'static str],
    header: &'static str,
    align: Align,
    about: &'static str,
}

const fn info(
    kw: Kw,
    names: &'static [&'static str],
    header: &'static str,
    align: Align,
    about: &'static str,
) -> Info {
    Info {
        kw,
        names,
        header,
        align,
        about,
    }
}

use Align::{Left as L, Right as R};

const KEYWORDS: &[Info] = &[
    info(
        Kw::Pcpu,
        &["%cpu", "pcpu", "cpu"],
        "%CPU",
        R,
        "percentage CPU usage",
    ),
    info(
        Kw::Pmem,
        &["%mem", "pmem", "mem"],
        "%MEM",
        R,
        "percentage memory usage",
    ),
    info(Kw::Args, &["args"], "ARGS", L, "command and arguments"),
    info(Kw::C, &["c"], "C", R, "CPU usage, whole percent"),
    info(Kw::Comm, &["comm"], "COMM", L, "command"),
    info(
        Kw::Command,
        &["command"],
        "COMMAND",
        L,
        "command and arguments",
    ),
    info(Kw::Etime, &["etime"], "ELAPSED", R, "elapsed running time"),
    info(
        Kw::Flags,
        &["flags", "f"],
        "F",
        R,
        "process flags, in hexadecimal",
    ),
    info(
        Kw::Lstart,
        &["lstart"],
        "STARTED",
        L,
        "time started, in full",
    ),
    info(Kw::Nice, &["nice", "ni"], "NI", R, "nice value"),
    info(
        Kw::Nlwp,
        &["nlwp", "thcount"],
        "NLWP",
        R,
        "number of threads",
    ),
    info(Kw::Pgid, &["pgid"], "PGID", R, "process group number"),
    info(Kw::Pid, &["pid"], "PID", R, "process ID"),
    info(Kw::Ppid, &["ppid"], "PPID", R, "parent process ID"),
    info(Kw::Pri, &["pri"], "PRI", R, "scheduling priority"),
    info(Kw::Rss, &["rss"], "RSS", R, "resident set size"),
    info(Kw::Ruid, &["ruid"], "RUID", R, "real user ID"),
    info(Kw::Ruser, &["ruser"], "RUSER", L, "user name (from ruid)"),
    info(Kw::Start, &["start", "stime"], "STARTED", L, "time started"),
    info(
        Kw::Stat,
        &["state", "stat"],
        "STAT",
        L,
        "symbolic process state",
    ),
    info(
        Kw::Time,
        &["time", "cputime"],
        "TIME",
        R,
        "accumulated CPU time, user + system",
    ),
    info(
        Kw::Tt,
        &["tt"],
        "TT",
        L,
        "control terminal name (abbreviated)",
    ),
    info(Kw::Tty, &["tty"], "TTY", L, "full name of control terminal"),
    info(
        Kw::Ucomm,
        &["ucomm"],
        "UCOMM",
        L,
        "name to be used for accounting",
    ),
    info(Kw::Uid, &["uid"], "UID", R, "effective user ID"),
    info(Kw::User, &["user"], "USER", L, "user name (from uid)"),
    info(Kw::Vsz, &["vsz", "vsize"], "VSZ", R, "virtual size"),
];

fn lookup(kw: Kw) -> &'static Info {
    KEYWORDS
        .iter()
        .find(|i| i.kw == kw)
        .expect("every Kw has an entry")
}

impl Kw {
    pub fn parse(name: &str) -> Option<Kw> {
        KEYWORDS
            .iter()
            .find(|i| i.names.contains(&name))
            .map(|i| i.kw)
    }
    pub fn header(self) -> &'static str {
        lookup(self).header
    }
    pub fn align(self) -> Align {
        lookup(self).align
    }
    /// Columns whose text is the command line, which pattern matches highlight.
    pub fn is_command(self) -> bool {
        matches!(self, Kw::Command | Kw::Args | Kw::Comm | Kw::Ucomm)
    }
}

/// `-L` output.
pub fn keyword_list() -> String {
    KEYWORDS
        .iter()
        .map(|i| format!("{:<10} {}\n", i.names.join(", "), i.about))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub kw: Kw,
    pub header: String,
}

impl Column {
    pub fn new(kw: Kw) -> Column {
        Column {
            kw,
            header: kw.header().to_string(),
        }
    }
    pub fn titled(kw: Kw, header: &str) -> Column {
        Column {
            kw,
            header: header.to_string(),
        }
    }
}

/// Everything a cell needs besides the process itself.
pub struct Ctx {
    pub total_memory: u64,
    pub now: i64,
    pub my_uid: u32,
    pub human: bool,
    /// `-c`: show only the executable name in COMMAND.
    pub comm_only: bool,
    /// Shorten long user names (macOS has daemon users up to 22 chars) so
    /// one of them doesn't widen the whole USER column. Terminal output only.
    pub short_users: bool,
}

const USER_WIDTH: usize = 16;

fn missing() -> Line {
    vec![Span::new("-", Role::Missing)]
}

fn one(text: impl Into<String>, role: Role) -> Line {
    vec![Span::new(text, role)]
}

fn size(bytes: Option<u64>, ctx: &Ctx, role: impl Fn(u64) -> Role) -> Line {
    match bytes {
        None => missing(),
        Some(b) if ctx.human => one(format::human(b), role(b)),
        Some(b) => one(format::kib(b), role(b)),
    }
}

fn pmem(p: &Process, ctx: &Ctx) -> Option<f64> {
    (ctx.total_memory > 0).then_some(())?;
    Some(p.rss? as f64 * 100.0 / ctx.total_memory as f64)
}

fn user(name: &str, uid: u32, ctx: &Ctx) -> Line {
    let kind = if uid == 0 {
        UserKind::Root
    } else if uid == ctx.my_uid {
        UserKind::Me
    } else {
        UserKind::Other(user_hash(name))
    };
    let role = Role::User(kind);
    if ctx.short_users && name.chars().count() > USER_WIDTH {
        let short: String = name.chars().take(USER_WIDTH - 1).collect();
        return one(short + "…", role);
    }
    one(name, role)
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// argv split into directory, executable, flags and arguments.
fn command(p: &Process, ctx: &Ctx) -> Line {
    let args = match &p.args {
        Some(a) if !a.is_empty() => a,
        _ => return one(format!("({})", p.comm), Role::CmdExe),
    };
    let argv0 = args[0].as_str();
    if ctx.comm_only {
        return one(basename(argv0), Role::CmdExe);
    }
    let exe = basename(argv0);
    let dir = &argv0[..argv0.len() - exe.len()];
    let mut line = Vec::with_capacity(args.len() * 2 + 1);
    if !dir.is_empty() {
        line.push(Span::new(dir, Role::CmdDir));
    }
    line.push(Span::new(exe, Role::CmdExe));
    for a in &args[1..] {
        line.push(Span::new(" ", Role::CmdArg));
        let role = if a.starts_with('-') && a.len() > 1 {
            Role::CmdFlag
        } else {
            Role::CmdArg
        };
        line.push(Span::new(a.as_str(), role));
    }
    line
}

pub fn cell(kw: Kw, p: &Process, ctx: &Ctx) -> Line {
    match kw {
        Kw::Pid => one(p.pid.to_string(), Role::Plain),
        Kw::Ppid => one(p.ppid.to_string(), Role::Plain),
        Kw::Pgid => one(p.pgid.to_string(), Role::Plain),
        Kw::Uid => one(p.uid.to_string(), Role::Plain),
        Kw::Ruid => one(p.ruid.to_string(), Role::Plain),
        Kw::User => user(&p.user, p.uid, ctx),
        Kw::Ruser => user(&p.ruser, p.ruid, ctx),
        Kw::Pcpu => p
            .pcpu
            .map_or_else(missing, |v| one(format!("{v:.1}"), Role::Pcpu(v))),
        Kw::C => p
            .pcpu
            .map_or_else(missing, |v| one(format!("{}", v as u64), Role::Pcpu(v))),
        Kw::Pmem => pmem(p, ctx).map_or_else(missing, |v| one(format!("{v:.1}"), Role::Pmem(v))),
        Kw::Rss => size(p.rss, ctx, Role::Rss),
        Kw::Vsz => size(p.vsz, ctx, |_| Role::Vsz),
        Kw::Tt => one(format::tt(p.tty.as_deref()), Role::Plain),
        Kw::Tty => one(p.tty.as_deref().unwrap_or("??"), Role::Plain),
        Kw::Stat => {
            let mut line = one(p.state.letter().to_string(), Role::State(p.state));
            let suffix = p.flags.suffix();
            if !suffix.is_empty() {
                line.push(Span::new(suffix, Role::StateFlags));
            }
            line
        }
        Kw::Start => one(format::start(p.start, ctx.now), Role::Plain),
        Kw::Lstart => one(format::lstart(p.start), Role::Plain),
        Kw::Etime => one(format::elapsed(ctx.now - p.start), Role::Plain),
        Kw::Time => p
            .cpu_time
            .map_or_else(missing, |t| one(format::cpu_time(t), Role::Plain)),
        Kw::Nice => one(p.nice.to_string(), Role::Plain),
        Kw::Pri => one(p.priority.to_string(), Role::Plain),
        Kw::Flags => one(format!("{:x}", p.raw_flags), Role::Plain),
        Kw::Nlwp => p
            .threads
            .map_or_else(missing, |n| one(n.to_string(), Role::Plain)),
        // Like Apple ps: comm is argv[0], ucomm the kernel's short name.
        Kw::Comm => match p.args.as_ref().and_then(|a| a.first()) {
            Some(argv0) => one(argv0.as_str(), Role::CmdExe),
            None => one(p.comm.as_str(), Role::CmdExe),
        },
        Kw::Ucomm => one(p.comm.as_str(), Role::CmdExe),
        Kw::Command | Kw::Args => command(p, ctx),
    }
}

/// A value to sort by. Missing values always sort last.
#[derive(Debug, PartialEq, PartialOrd)]
pub enum SortVal {
    Num(f64),
    Text(String),
}

pub fn sort_value(kw: Kw, p: &Process, ctx: &Ctx) -> Option<SortVal> {
    use SortVal::{Num, Text};
    Some(match kw {
        Kw::Pid => Num(p.pid as f64),
        Kw::Ppid => Num(p.ppid as f64),
        Kw::Pgid => Num(p.pgid as f64),
        Kw::Uid => Num(p.uid as f64),
        Kw::Ruid => Num(p.ruid as f64),
        Kw::User => Text(p.user.clone()),
        Kw::Ruser => Text(p.ruser.clone()),
        Kw::Pcpu | Kw::C => Num(p.pcpu?),
        Kw::Pmem => Num(pmem(p, ctx)?),
        Kw::Rss => Num(p.rss? as f64),
        Kw::Vsz => Num(p.vsz? as f64),
        Kw::Tt | Kw::Tty => Text(p.tty.clone()?),
        Kw::Stat => Text(p.state.letter().to_string()),
        Kw::Start | Kw::Lstart => Num(p.start as f64),
        Kw::Etime => Num((ctx.now - p.start) as f64),
        Kw::Time => Num(p.cpu_time?.as_secs_f64()),
        Kw::Nice => Num(p.nice as f64),
        Kw::Pri => Num(p.priority as f64),
        Kw::Flags => Num(p.raw_flags as f64),
        Kw::Nlwp => Num(p.threads? as f64),
        Kw::Comm | Kw::Ucomm => Text(p.comm.to_lowercase()),
        Kw::Command | Kw::Args => Text(p.command_line().to_lowercase()),
    })
}

/// Compare two optional sort values, keeping `None` last whatever the direction.
pub fn compare(a: Option<SortVal>, b: Option<SortVal>, descending: bool) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(a), Some(b)) => {
            let o = a.partial_cmp(&b).unwrap_or(Ordering::Equal);
            if descending { o.reverse() } else { o }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::plain_text;

    fn ctx() -> Ctx {
        Ctx {
            total_memory: 16 << 30,
            now: 1_000_000,
            my_uid: 501,
            human: false,
            comm_only: false,
            short_users: false,
        }
    }

    fn proc() -> Process {
        Process {
            pid: 42,
            uid: 501,
            user: "guy".into(),
            comm: "node".into(),
            args: Some(vec![
                "/usr/local/bin/node".into(),
                "--inspect".into(),
                "app.js".into(),
            ]),
            rss: Some(1 << 30),
            ..Default::default()
        }
    }

    #[test]
    fn every_keyword_has_info_and_parses_by_alias() {
        for i in KEYWORDS {
            for n in i.names {
                assert_eq!(Kw::parse(n), Some(i.kw));
            }
        }
        assert_eq!(Kw::parse("nope"), None);
    }

    #[test]
    fn command_is_split_into_roles() {
        let line = cell(Kw::Command, &proc(), &ctx());
        let roles: Vec<_> = line.iter().map(|s| s.role).collect();
        assert_eq!(
            roles,
            [
                Role::CmdDir,
                Role::CmdExe,
                Role::CmdArg,
                Role::CmdFlag,
                Role::CmdArg,
                Role::CmdArg
            ]
        );
        assert_eq!(plain_text(&line), "/usr/local/bin/node --inspect app.js");
    }

    #[test]
    fn comm_only_and_unreadable_args() {
        let c = Ctx {
            comm_only: true,
            ..ctx()
        };
        assert_eq!(plain_text(&cell(Kw::Command, &proc(), &c)), "node");
        let p = Process {
            args: None,
            comm: "launchd".into(),
            ..proc()
        };
        assert_eq!(plain_text(&cell(Kw::Command, &p, &ctx())), "(launchd)");
    }

    #[test]
    fn sizes_raw_or_human_and_pmem() {
        assert_eq!(plain_text(&cell(Kw::Rss, &proc(), &ctx())), "1048576");
        let h = Ctx {
            human: true,
            ..ctx()
        };
        assert_eq!(plain_text(&cell(Kw::Rss, &proc(), &h)), "1.0G");
        assert_eq!(plain_text(&cell(Kw::Pmem, &proc(), &ctx())), "6.2");
        let p = Process {
            rss: None,
            ..proc()
        };
        assert_eq!(
            cell(Kw::Rss, &p, &ctx()),
            vec![Span::new("-", Role::Missing)]
        );
    }

    #[test]
    fn long_user_names_shorten_only_when_asked() {
        let p = Process {
            uid: 266,
            user: "_reportmemoryexception".into(),
            ..proc()
        };
        assert_eq!(
            plain_text(&cell(Kw::User, &p, &ctx())),
            "_reportmemoryexception"
        );
        let c = Ctx {
            short_users: true,
            ..ctx()
        };
        assert_eq!(plain_text(&cell(Kw::User, &p, &c)), "_reportmemoryex…");
    }

    #[test]
    fn missing_sorts_last_both_directions() {
        let some = || Some(SortVal::Num(1.0));
        assert_eq!(compare(None, some(), false), Ordering::Greater);
        assert_eq!(compare(None, some(), true), Ordering::Greater);
        assert_eq!(
            compare(Some(SortVal::Num(2.0)), some(), true),
            Ordering::Less
        );
    }
}
