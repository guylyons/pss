//! Command-line parsing. Hand-written because `ps` mixes BSD option bundles
//! (`aux`, no dash) with dash flags, which argument-parsing crates reject.

use crate::columns::{Column, Kw};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorMode {
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Preset {
    /// `PID TTY TIME CMD`
    #[default]
    Default,
    /// `-f`
    Full,
    /// BSD `u`
    User,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortKey {
    pub kw: Kw,
    pub descending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Action {
    #[default]
    Run,
    Help,
    Version,
    ListKeywords,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Options {
    pub action: Action,
    /// `-a`: other users' processes too.
    pub all_users: bool,
    /// `-x`: processes without a controlling terminal too.
    pub no_tty: bool,
    /// `-A` / `-e`: everything.
    pub all: bool,
    pub pids: Vec<i32>,
    /// `-u`: effective user names or uids.
    pub users: Vec<String>,
    /// `-U`: real user names or uids.
    pub real_users: Vec<String>,
    pub ttys: Vec<String>,
    pub preset: Preset,
    /// `-o`: replaces the preset when non-empty.
    pub custom: Vec<Column>,
    /// `-O`: added after PID.
    pub extra: Vec<Column>,
    pub sort: Vec<SortKey>,
    pub comm_only: bool,
    /// Number of `w` flags.
    pub wide: u8,
    pub patterns: Vec<String>,
    pub color: ColorMode,
    pub no_pager: bool,
    /// `Some(true)` = `--human`, `Some(false)` = `--raw`, `None` = decide by tty.
    pub human: Option<bool>,
}

impl Options {
    /// The columns to show, in order.
    pub fn columns(&self) -> Vec<Column> {
        let mut cols = if !self.custom.is_empty() {
            self.custom.clone()
        } else {
            match self.preset {
                Preset::Default => vec![
                    Column::new(Kw::Pid),
                    Column::new(Kw::Tty),
                    Column::new(Kw::Time),
                    Column::titled(Kw::Command, "CMD"),
                ],
                Preset::Full => vec![
                    Column::new(Kw::Uid),
                    Column::new(Kw::Pid),
                    Column::new(Kw::Ppid),
                    Column::new(Kw::C),
                    Column::titled(Kw::Start, "STIME"),
                    Column::new(Kw::Tty),
                    Column::new(Kw::Time),
                    Column::titled(Kw::Command, "CMD"),
                ],
                Preset::User => [
                    Kw::User,
                    Kw::Pid,
                    Kw::Pcpu,
                    Kw::Pmem,
                    Kw::Vsz,
                    Kw::Rss,
                    Kw::Tt,
                    Kw::Stat,
                    Kw::Start,
                    Kw::Time,
                    Kw::Command,
                ]
                .into_iter()
                .map(Column::new)
                .collect(),
            }
        };
        if !self.extra.is_empty() {
            let at = cols
                .iter()
                .position(|c| c.kw == Kw::Pid)
                .map_or(0, |i| i + 1);
            cols.splice(at..at, self.extra.iter().cloned());
        }
        cols
    }
}

/// Letters allowed in a dash-less BSD bundle like `aux`. `o` takes the next argument.
const BUNDLE_LETTERS: &str = "auxwcrmAo";

fn is_bundle(arg: &str) -> bool {
    !arg.is_empty() && arg.chars().all(|c| BUNDLE_LETTERS.contains(c))
}

/// `-aux` is a habit from Linux; treat it as the bundle `aux` instead of
/// `-a -u x` (macOS ps would look for a user named "x").
fn is_dashed_bundle(body: &str) -> bool {
    body.chars().all(|c| "auxw".contains(c)) && body.contains('u') && !body.ends_with('u')
}

pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Options, String> {
    let mut o = Options::default();
    let mut it = args.into_iter().peekable();
    let mut seen_bare = false;

    while let Some(arg) = it.next() {
        if arg == "--" {
            o.patterns.extend(it.by_ref());
            break;
        }
        if let Some(long) = arg.strip_prefix("--") {
            let (name, inline) = match long.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (long, None),
            };
            match name {
                "help" => o.action = Action::Help,
                "version" => o.action = Action::Version,
                "no-pager" => o.no_pager = true,
                "human" => o.human = Some(true),
                "raw" => o.human = Some(false),
                "color" | "colour" => {
                    o.color = match inline.as_deref() {
                        None | Some("always") => ColorMode::Always,
                        Some("auto") => ColorMode::Auto,
                        Some("never") => ColorMode::Never,
                        Some(v) => {
                            return Err(format!(
                                "invalid --color value '{v}' (use auto, always or never)"
                            ));
                        }
                    }
                }
                "sort" => {
                    let v = match inline {
                        Some(v) => v,
                        None => it.next().ok_or("--sort needs a keyword list")?,
                    };
                    o.sort.extend(parse_sort(&v)?);
                }
                _ => return Err(format!("unknown option --{name}")),
            }
            continue;
        }
        if let Some(body) = arg.strip_prefix('-').filter(|b| !b.is_empty()) {
            if is_dashed_bundle(body) {
                apply_bundle(&mut o, body, &mut it)?;
            } else {
                parse_dash(&mut o, body, &mut it)?;
            }
            continue;
        }
        if !seen_bare && is_bundle(&arg) {
            apply_bundle(&mut o, &arg, &mut it)?;
        } else {
            o.patterns.push(arg);
        }
        seen_bare = true;
    }
    Ok(o)
}

fn apply_bundle(
    o: &mut Options,
    bundle: &str,
    it: &mut impl Iterator<Item = String>,
) -> Result<(), String> {
    for c in bundle.chars() {
        match c {
            'a' => o.all_users = true,
            'u' => o.preset = Preset::User,
            'x' => o.no_tty = true,
            'w' => o.wide = o.wide.saturating_add(1),
            'c' => o.comm_only = true,
            'r' => {
                o.sort = vec![SortKey {
                    kw: Kw::Pcpu,
                    descending: true,
                }]
            }
            'm' => {
                o.sort = vec![SortKey {
                    kw: Kw::Rss,
                    descending: true,
                }]
            }
            'A' => o.all = true,
            'o' => {
                let v = it.next().ok_or("option o needs a keyword list")?;
                o.custom.extend(parse_columns(&v)?);
            }
            _ => unreachable!("is_bundle checked the letters"),
        }
    }
    Ok(())
}

fn parse_dash(
    o: &mut Options,
    body: &str,
    it: &mut impl Iterator<Item = String>,
) -> Result<(), String> {
    for (i, c) in body.char_indices() {
        // Options taking a value use the rest of this argument, or the next one.
        let mut value = || -> Result<String, String> {
            let rest = &body[i + c.len_utf8()..];
            if !rest.is_empty() {
                Ok(rest.to_string())
            } else {
                it.next()
                    .ok_or_else(|| format!("option -{c} requires an argument"))
            }
        };
        match c {
            'a' => o.all_users = true,
            'A' | 'e' => o.all = true,
            'x' => o.no_tty = true,
            'c' => o.comm_only = true,
            'f' => o.preset = Preset::Full,
            'r' => {
                o.sort = vec![SortKey {
                    kw: Kw::Pcpu,
                    descending: true,
                }]
            }
            'm' => {
                o.sort = vec![SortKey {
                    kw: Kw::Rss,
                    descending: true,
                }]
            }
            'w' => o.wide = o.wide.saturating_add(1),
            'L' => o.action = Action::ListKeywords,
            'P' => o.no_pager = true,
            'h' => o.action = Action::Help,
            'V' => o.action = Action::Version,
            'o' => {
                o.custom.extend(parse_columns(&value()?)?);
                return Ok(());
            }
            'O' => {
                o.extra.extend(parse_columns(&value()?)?);
                return Ok(());
            }
            'p' => {
                for p in split_list(&value()?) {
                    o.pids
                        .push(p.parse().map_err(|_| format!("invalid process id: {p}"))?);
                }
                return Ok(());
            }
            'u' => {
                o.users.extend(split_list(&value()?).map(String::from));
                return Ok(());
            }
            'U' => {
                o.real_users.extend(split_list(&value()?).map(String::from));
                return Ok(());
            }
            't' => {
                o.ttys.extend(split_list(&value()?).map(String::from));
                return Ok(());
            }
            _ => return Err(format!("illegal option -- {c}")),
        }
    }
    Ok(())
}

fn split_list(v: &str) -> impl Iterator<Item = &str> {
    v.split([',', ' ']).filter(|s| !s.is_empty())
}

/// `pid,user=WHO,command`: `kw=HEADER` renames a column; an empty header
/// hides it, and if every header is empty there is no header line.
pub fn parse_columns(v: &str) -> Result<Vec<Column>, String> {
    let cols = split_list(v)
        .map(|item| {
            let (name, header) = match item.split_once('=') {
                Some((n, h)) => (n, Some(h)),
                None => (item, None),
            };
            let kw = Kw::parse(name).ok_or_else(|| format!("{name}: keyword not found"))?;
            Ok(header.map_or_else(|| Column::new(kw), |h| Column::titled(kw, h)))
        })
        .collect::<Result<Vec<_>, String>>()?;
    if cols.is_empty() {
        return Err("empty keyword list".into());
    }
    Ok(cols)
}

fn parse_sort(v: &str) -> Result<Vec<SortKey>, String> {
    split_list(v)
        .map(|k| {
            let (descending, name) = match k.strip_prefix('-') {
                Some(n) => (true, n),
                None => (false, k.strip_prefix('+').unwrap_or(k)),
            };
            let kw = Kw::parse(name).ok_or_else(|| format!("{name}: keyword not found"))?;
            Ok(SortKey { kw, descending })
        })
        .collect()
}

pub const HELP: &str = "\
pss — a readable ps

Usage: pss [BSD-OPTIONS] [-options] [--long-options] [PATTERN...]

Selection (default: your processes that have a terminal):
  a, -a            include other users' processes
  x, -x            include processes without a controlling terminal
  -A, -e           all processes
  -p PIDS          only these process IDs
  -u USERS         processes of these (effective) users
  -U USERS         processes of these real users
  -t TTYS          processes on these terminals (s000, ttys000)
  PATTERN...       match command lines (lowercase = ignore case); implies -A,
                   highlights the match, and leaves pss itself out.
                   Use `pss -- aux` to search for text that looks like a flag bundle.

Output:
  u                user-oriented columns (as in `ps aux`)
  -f               full format: UID PID PPID C STIME TTY TIME CMD
  -o KEYWORDS      choose columns (kw=HEADER renames); o in a bundle: `pss axo pid,comm`
  -O KEYWORDS      add columns after PID
  -c               show only the executable name in COMMAND
  -w, -ww          wider / unlimited output on a terminal
  -r, -m           sort by CPU / by memory, highest first
  --sort KEYS      sort by keywords; prefix with - for descending (--sort -%cpu,pid)
  -L               list keywords

Display:
  --color[=WHEN]   auto (default), always, never; NO_COLOR also disables color
  -P, --no-pager   never open the pager (or set PSS_PAGER=0)
  --human, --raw   sizes as 1.2G, or in KiB like ps (default: human on a terminal)
  -h, --help       this help
  -V, --version    version

Pager keys: j/k ↑/↓ scroll, space/b page, g/G top/bottom, ←/→ pan,
            / search, n/N next/previous match, q quit.

Fields shown as - need root on macOS (try `sudo pss aux`).
";

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<Options, String> {
        parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn bsd_aux() {
        let o = p(&["aux"]).unwrap();
        assert!(o.all_users && o.no_tty);
        assert_eq!(o.preset, Preset::User);
        assert_eq!(o.columns()[0].kw, Kw::User);
        assert_eq!(p(&["-aux"]).unwrap(), o);
    }

    #[test]
    fn bare_words_after_bundle_are_patterns() {
        let o = p(&["aux", "node", "--sort", "-cpu"]).unwrap();
        assert_eq!(o.patterns, vec!["node"]);
        assert_eq!(
            o.sort,
            vec![SortKey {
                kw: Kw::Pcpu,
                descending: true
            }]
        );
    }

    #[test]
    fn non_bundle_word_is_a_pattern_and_double_dash_forces_it() {
        assert_eq!(p(&["node"]).unwrap().patterns, vec!["node"]);
        let o = p(&["--", "aux"]).unwrap();
        assert_eq!(o.patterns, vec!["aux"]);
        assert!(!o.all_users);
    }

    #[test]
    fn dash_flags_with_values() {
        let o = p(&["-ef", "-p", "1,2", "-uroot", "-t", "s000"]).unwrap();
        assert!(o.all);
        assert_eq!(o.preset, Preset::Full);
        assert_eq!(o.pids, vec![1, 2]);
        assert_eq!(o.users, vec!["root"]);
        assert_eq!(o.ttys, vec!["s000"]);
    }

    #[test]
    fn custom_columns_and_renamed_header() {
        let o = p(&["-o", "pid,user", "-o", "command=WHAT,ppid="]).unwrap();
        let cols = o.columns();
        assert_eq!(cols.len(), 4);
        assert_eq!(cols[2], Column::titled(Kw::Command, "WHAT"));
        assert_eq!(cols[3], Column::titled(Kw::Ppid, ""));
        let o = p(&["axo", "pid,comm"]).unwrap();
        assert_eq!(
            o.columns(),
            vec![Column::new(Kw::Pid), Column::new(Kw::Comm)]
        );
    }

    #[test]
    fn extra_columns_go_after_pid() {
        let o = p(&["-O", "ppid"]).unwrap();
        let kws: Vec<_> = o.columns().iter().map(|c| c.kw).collect();
        assert_eq!(kws, [Kw::Pid, Kw::Ppid, Kw::Tty, Kw::Time, Kw::Command]);
    }

    #[test]
    fn errors() {
        assert_eq!(
            p(&["-o", "pid,bogus"]).unwrap_err(),
            "bogus: keyword not found"
        );
        assert_eq!(p(&["-Z"]).unwrap_err(), "illegal option -- Z");
        assert!(p(&["-p"]).is_err());
        assert!(p(&["-p", "abc"]).is_err());
        assert!(p(&["--color=sometimes"]).is_err());
    }

    #[test]
    fn display_flags() {
        let o = p(&["-P", "--raw", "--color=never", "-ww"]).unwrap();
        assert!(o.no_pager);
        assert_eq!(o.human, Some(false));
        assert_eq!(o.color, ColorMode::Never);
        assert_eq!(o.wide, 2);
        assert_eq!(p(&["--color"]).unwrap().color, ColorMode::Always);
    }
}
