mod cli;
mod columns;
mod format;
mod output;
mod process;
mod select;
mod source;
mod style;
mod table;
mod text;
mod users;

use std::io::{self, BufWriter, IsTerminal};
use std::process::ExitCode;

use cli::{Action, ColorMode, Options};
use columns::Ctx;
use select::Selectors;
use text::Matcher;

fn main() -> ExitCode {
    let opts = match cli::parse(std::env::args().skip(1)) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("pss: {e}\nTry 'pss --help' for more information.");
            return ExitCode::FAILURE;
        }
    };
    match opts.action {
        Action::Help => print!("{}", cli::HELP),
        Action::Version => println!("pss {}", env!("CARGO_PKG_VERSION")),
        Action::ListKeywords => print!("{}", columns::keyword_list()),
        Action::Run => {
            return match run(&opts) {
                Ok(code) => code,
                Err(e) if e.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("pss: {e}");
                    ExitCode::FAILURE
                }
            };
        }
    }
    ExitCode::SUCCESS
}

fn env_set(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|v| !v.is_empty())
}

fn run(o: &Options) -> io::Result<ExitCode> {
    let sel = Selectors::resolve(o).map_err(io::Error::other)?;
    let tty = io::stdout().is_terminal();
    let color = match o.color {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => {
            tty && !env_set("NO_COLOR") && std::env::var("TERM").map_or(true, |t| t != "dumb")
        }
    };

    let snap = source::system().snapshot()?;
    let ctx = Ctx {
        total_memory: snap.total_memory,
        now: snap.now,
        my_uid: snap.my_uid,
        human: o.human.unwrap_or(tty),
        comm_only: o.comm_only,
        short_users: tty,
    };
    let matcher = Matcher::new(&o.patterns);
    let mut procs = select::select(&snap, o, &sel, matcher.as_ref());
    select::sort(&mut procs, &o.sort, &ctx);
    let table = table::build(&procs, &o.columns(), &ctx, matcher.as_ref());

    let (cols, rows) = ratatui::crossterm::terminal::size().unwrap_or((80, 24));
    let page = tty && !o.no_pager && std::env::var("PSS_PAGER").map_or(true, |v| v != "0");
    if page && table.rows.len() + 1 >= rows as usize {
        output::pager::run(&table, color)?;
    } else {
        let width = match (tty, o.wide) {
            (false, _) | (_, 2..) => None,
            (true, 0) => Some(cols as usize),
            (true, 1) => Some((cols as usize).max(132)),
        };
        output::plain::write(
            &mut BufWriter::new(io::stdout().lock()),
            &table,
            color,
            width,
        )?;
    }

    // Like pgrep: searching for something that isn't there is a failure.
    let searched = matcher.is_some()
        || !o.pids.is_empty()
        || !o.users.is_empty()
        || !o.real_users.is_empty()
        || !o.ttys.is_empty();
    Ok(if searched && procs.is_empty() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}
