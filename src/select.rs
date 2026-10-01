//! Which processes to show, and in what order.

use crate::cli::{Options, SortKey};
use crate::columns::{Ctx, compare, sort_value};
use crate::process::{Process, Snapshot};
use crate::text::Matcher;
use crate::users::uid_for_name;

/// `-u`/`-U`/`-p`/`-t` resolved against the system.
#[derive(Debug, Default)]
pub struct Selectors {
    pub pids: Vec<i32>,
    pub uids: Vec<u32>,
    pub ruids: Vec<u32>,
    pub ttys: Vec<String>,
}

impl Selectors {
    pub fn resolve(o: &Options) -> Result<Selectors, String> {
        let uids = |names: &[String]| -> Result<Vec<u32>, String> {
            names
                .iter()
                .map(|n| {
                    uid_for_name(n)
                        .or_else(|| n.parse().ok())
                        .ok_or_else(|| format!("unknown user: {n}"))
                })
                .collect()
        };
        Ok(Selectors {
            pids: o.pids.clone(),
            uids: uids(&o.users)?,
            ruids: uids(&o.real_users)?,
            ttys: o.ttys.iter().map(|t| normalize_tty(t)).collect(),
        })
    }

    fn any(&self) -> bool {
        !(self.pids.is_empty()
            && self.uids.is_empty()
            && self.ruids.is_empty()
            && self.ttys.is_empty())
    }

    fn matches(&self, p: &Process) -> bool {
        self.pids.contains(&p.pid)
            || self.uids.contains(&p.uid)
            || self.ruids.contains(&p.ruid)
            || p.tty.as_ref().is_some_and(|t| self.ttys.contains(t))
    }
}

/// `s000`, `ttys000` and `/dev/ttys000` all mean `ttys000`.
fn normalize_tty(t: &str) -> String {
    let t = t.strip_prefix("/dev/").unwrap_or(t);
    if t.starts_with("tty") || t == "console" {
        t.to_string()
    } else {
        format!("tty{t}")
    }
}

pub fn select<'a>(
    snap: &'a Snapshot,
    o: &Options,
    sel: &Selectors,
    matcher: Option<&Matcher>,
) -> Vec<&'a Process> {
    snap.processes
        .iter()
        .filter(|p| {
            let base = if sel.any() {
                sel.matches(p)
            } else if o.all || matcher.is_some() {
                true
            } else {
                (o.all_users || p.uid == snap.my_uid) && (o.no_tty || p.tty.is_some())
            };
            base && matcher.is_none_or(|m| p.pid != snap.my_pid && m.is_match(&p.command_line()))
        })
        .collect()
}

/// Sort by the given keys; with none, by terminal then PID like ps.
pub fn sort(procs: &mut [&Process], keys: &[SortKey], ctx: &Ctx) {
    if keys.is_empty() {
        procs.sort_by(|a, b| (&a.tty, a.pid).cmp(&(&b.tty, b.pid)));
        return;
    }
    procs.sort_by(|a, b| {
        keys.iter()
            .map(|k| {
                compare(
                    sort_value(k.kw, a, ctx),
                    sort_value(k.kw, b, ctx),
                    k.descending,
                )
            })
            .find(|o| o.is_ne())
            .unwrap_or_else(|| a.pid.cmp(&b.pid))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::parse;
    use crate::columns::Kw;

    fn proc(pid: i32, uid: u32, tty: Option<&str>, cmd: &str) -> Process {
        Process {
            pid,
            uid,
            ruid: uid,
            tty: tty.map(String::from),
            comm: cmd.into(),
            args: Some(cmd.split(' ').map(String::from).collect()),
            ..Default::default()
        }
    }

    fn snap() -> Snapshot {
        Snapshot {
            processes: vec![
                proc(1, 0, None, "/sbin/launchd"),
                proc(10, 501, Some("ttys000"), "-zsh"),
                proc(11, 501, None, "node server.js"),
                proc(12, 502, Some("ttys001"), "vim notes"),
                proc(99, 501, Some("ttys000"), "pss node"),
            ],
            my_uid: 501,
            my_pid: 99,
            ..Default::default()
        }
    }

    fn pids(args: &[&str]) -> Vec<i32> {
        let o = parse(args.iter().map(|s| s.to_string())).unwrap();
        let sel = Selectors::resolve(&o).unwrap();
        let m = Matcher::new(&o.patterns);
        let s = snap();
        select(&s, &o, &sel, m.as_ref())
            .iter()
            .map(|p| p.pid)
            .collect()
    }

    #[test]
    fn default_is_mine_with_tty() {
        assert_eq!(pids(&[]), [10, 99]);
    }

    #[test]
    fn a_and_x_widen() {
        assert_eq!(pids(&["a"]), [10, 12, 99]);
        assert_eq!(pids(&["x"]), [10, 11, 99]);
        assert_eq!(pids(&["ax"]), [1, 10, 11, 12, 99]);
        assert_eq!(pids(&["-A"]), [1, 10, 11, 12, 99]);
    }

    #[test]
    fn explicit_selectors_union() {
        assert_eq!(pids(&["-p", "1", "-t", "s001"]), [1, 12]);
        assert_eq!(pids(&["-u", "0"]), [1]);
    }

    #[test]
    fn pattern_searches_everything_but_itself() {
        assert_eq!(pids(&["node"]), [11]);
        assert_eq!(pids(&["LAUNCHD"]), Vec::<i32>::new());
        assert_eq!(pids(&["launchd"]), [1]);
    }

    #[test]
    fn sort_keys_then_pid() {
        let s = snap();
        let mut v: Vec<&Process> = s.processes.iter().collect();
        let ctx = Ctx {
            total_memory: 0,
            now: 0,
            my_uid: 501,
            human: false,
            comm_only: false,
            short_users: false,
        };
        sort(
            &mut v,
            &[SortKey {
                kw: Kw::Uid,
                descending: true,
            }],
            &ctx,
        );
        assert_eq!(
            v.iter().map(|p| p.pid).collect::<Vec<_>>(),
            [12, 10, 11, 99, 1]
        );
        sort(&mut v, &[], &ctx);
        assert_eq!(
            v.iter().map(|p| p.pid).collect::<Vec<_>>(),
            [1, 11, 10, 99, 12]
        );
    }
}
