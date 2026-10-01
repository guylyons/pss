//! Collecting and laying out the process list, again and again: once for
//! printing, and on every refresh in the interactive view.

use std::io;

use crate::cli::Options;
use crate::columns::Ctx;
use crate::output::pager::Host;
use crate::select::{self, Selectors};
use crate::signal::{self, Signal};
use crate::source::{self, ProcessSource};
use crate::table::{self, Table};
use crate::text::Matcher;

pub struct Collector {
    source: Box<dyn ProcessSource>,
    opts: Options,
    sel: Selectors,
    matcher: Option<Matcher>,
    /// Output goes to a terminal (human sizes, short user names).
    tty: bool,
}

impl Collector {
    pub fn new(opts: Options, tty: bool) -> Result<Collector, String> {
        Ok(Collector {
            source: source::system(),
            sel: Selectors::resolve(&opts)?,
            matcher: Matcher::new(&opts.patterns),
            opts,
            tty,
        })
    }

    /// Whether the options ask for specific processes, so finding none is
    /// a failure (like pgrep).
    pub fn searched(&self) -> bool {
        let o = &self.opts;
        self.matcher.is_some()
            || !o.pids.is_empty()
            || !o.users.is_empty()
            || !o.real_users.is_empty()
            || !o.ttys.is_empty()
    }

    pub fn collect(&mut self) -> io::Result<Table> {
        let snap = self.source.snapshot()?;
        let o = &self.opts;
        let ctx = Ctx {
            total_memory: snap.total_memory,
            now: snap.now,
            my_uid: snap.my_uid,
            human: o.human.unwrap_or(self.tty),
            comm_only: o.comm_only,
            short_users: self.tty,
        };
        let mut procs = select::select(&snap, o, &self.sel, self.matcher.as_ref());
        select::sort(&mut procs, &o.sort, &ctx);
        Ok(table::build(
            &procs,
            &o.columns(),
            &ctx,
            self.matcher.as_ref(),
        ))
    }
}

impl Host for Collector {
    fn reload(&mut self) -> io::Result<Table> {
        self.collect()
    }

    fn kill(&mut self, pid: i32, sig: Signal) -> Result<(), String> {
        // Dying here would leave the terminal stuck in the full-screen view.
        if pid == std::process::id() as i32 {
            return Err("That's pss itself; press q to quit".into());
        }
        signal::send(pid, sig)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::parse;

    fn collector(args: &[&str]) -> Collector {
        Collector::new(parse(args.iter().map(|s| s.to_string())).unwrap(), false).unwrap()
    }

    #[test]
    fn reload_uses_the_same_options() {
        let me = std::process::id().to_string();
        let mut c = collector(&["-p", &me, "-o", "pid="]);
        let t = c.reload().unwrap();
        let pids: Vec<i32> = t.ids.iter().map(|r| r.pid).collect();
        assert_eq!(pids, vec![std::process::id() as i32]);
    }

    #[test]
    fn refuses_to_kill_itself() {
        let mut c = collector(&[]);
        let err = c.kill(std::process::id() as i32, Signal::Term).unwrap_err();
        assert_eq!(err, "That's pss itself; press q to quit");
    }

    #[test]
    fn unknown_user_is_an_error() {
        let o = parse(["-u".to_string(), "no-such-user-pss".to_string()]).unwrap();
        assert!(Collector::new(o, false).is_err());
    }
}
