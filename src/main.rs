mod app;
mod cli;
mod columns;
mod format;
mod output;
mod process;
mod select;
mod signal;
mod source;
mod style;
mod table;
mod text;
mod users;

use std::io::{self, BufWriter, IsTerminal};
use std::process::ExitCode;

use app::Collector;
use cli::{Action, ColorMode, Options};

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
    let tty = io::stdout().is_terminal();
    let color = match o.color {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => {
            tty && !env_set("NO_COLOR") && std::env::var("TERM").map_or(true, |t| t != "dumb")
        }
    };

    let mut app = Collector::new(o.clone(), tty).map_err(io::Error::other)?;
    let table = app.collect()?;
    // Like pgrep: searching for something that isn't there is a failure.
    let code = if app.searched() && table.rows.is_empty() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    };

    let (cols, rows) = ratatui::crossterm::terminal::size().unwrap_or((80, 24));
    let page = tty && !o.no_pager && std::env::var("PSS_PAGER").map_or(true, |v| v != "0");
    if page && (o.interactive || table.rows.len() + 1 >= rows as usize) {
        output::pager::run(table, color, &mut app)?;
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
    Ok(code)
}
