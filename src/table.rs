//! Laying cells out into aligned, styled lines.

use unicode_width::UnicodeWidthStr;

use crate::columns::{Align, Column, Ctx, cell};
use crate::process::Process;
use crate::style::Role;
use crate::text::{Line, Matcher, Span, highlight, plain_text};

#[derive(Debug, Clone, Default)]
pub struct Table {
    /// Empty when there is no header line.
    pub header: Line,
    pub rows: Vec<Line>,
}

pub fn build(procs: &[&Process], cols: &[Column], ctx: &Ctx, matcher: Option<&Matcher>) -> Table {
    let cells: Vec<Vec<Line>> = procs
        .iter()
        .map(|p| {
            cols.iter()
                .map(|c| {
                    let line = cell(c.kw, p, ctx);
                    match matcher {
                        Some(m) if c.kw.is_command() => {
                            let ranges = m.find(&plain_text(&line));
                            highlight(line, &ranges, Role::Match)
                        }
                        _ => line,
                    }
                })
                .collect()
        })
        .collect();

    let widths: Vec<usize> = cols
        .iter()
        .enumerate()
        .map(|(i, c)| {
            cells
                .iter()
                .map(|row| width(&row[i]))
                .chain([c.header.width()])
                .max()
                .unwrap_or(0)
        })
        .collect();

    // As in ps, `-o pid=` with every header empty means no header line.
    let header = if cols.iter().all(|c| c.header.is_empty()) {
        Vec::new()
    } else {
        let cells = cols
            .iter()
            .map(|c| vec![Span::new(c.header.as_str(), Role::Header)])
            .collect();
        join(cells, cols, &widths)
    };
    Table {
        header,
        rows: cells
            .into_iter()
            .map(|row| join(row, cols, &widths))
            .collect(),
    }
}

fn width(line: &[Span]) -> usize {
    line.iter().map(|s| s.text.width()).sum()
}

/// Pad cells to their column widths and join with single spaces. A
/// left-aligned last column gets no trailing padding.
fn join(row: Vec<Line>, cols: &[Column], widths: &[usize]) -> Line {
    let last = row.len().saturating_sub(1);
    let mut out = Vec::new();
    for (i, cell) in row.into_iter().enumerate() {
        if i > 0 {
            out.push(Span::new(" ", Role::Plain));
        }
        let pad = widths[i].saturating_sub(width(&cell));
        match cols[i].kw.align() {
            Align::Right => {
                if pad > 0 {
                    out.push(Span::new(" ".repeat(pad), Role::Plain));
                }
                out.extend(cell);
            }
            Align::Left => {
                out.extend(cell);
                if pad > 0 && i != last {
                    out.push(Span::new(" ".repeat(pad), Role::Plain));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::columns::Kw;

    fn ctx() -> Ctx {
        Ctx {
            total_memory: 0,
            now: 0,
            my_uid: 501,
            human: false,
            comm_only: false,
            short_users: false,
        }
    }

    #[test]
    fn aligns_numbers_right_and_text_left() {
        let a = Process {
            pid: 7,
            user: "root".into(),
            comm: "a".into(),
            args: Some(vec!["a".into()]),
            ..Default::default()
        };
        let b = Process {
            pid: 12345,
            user: "guy".into(),
            comm: "bb".into(),
            args: Some(vec!["bb".into()]),
            ..Default::default()
        };
        let cols = [
            Column::new(Kw::Pid),
            Column::new(Kw::User),
            Column::new(Kw::Command),
        ];
        let t = build(&[&a, &b], &cols, &ctx(), None);
        assert_eq!(plain_text(&t.header), "  PID USER COMMAND");
        assert_eq!(plain_text(&t.rows[0]), "    7 root a");
        assert_eq!(plain_text(&t.rows[1]), "12345 guy  bb");
    }

    #[test]
    fn pattern_highlights_only_command_columns() {
        let p = Process {
            pid: 1,
            user: "node".into(),
            comm: "node".into(),
            args: Some(vec!["node".into()]),
            ..Default::default()
        };
        let cols = [Column::new(Kw::User), Column::new(Kw::Command)];
        let m = Matcher::new(&["node"]).unwrap();
        let t = build(&[&p], &cols, &ctx(), Some(&m));
        let matched: Vec<_> = t.rows[0].iter().filter(|s| s.role == Role::Match).collect();
        assert_eq!(matched.len(), 1);
    }

    #[test]
    fn all_empty_headers_mean_no_header() {
        let p = Process {
            pid: 3,
            ..Default::default()
        };
        let t = build(&[&p], &[Column::titled(Kw::Pid, "")], &ctx(), None);
        assert!(t.header.is_empty());
        assert_eq!(plain_text(&t.rows[0]), "3");
    }
}
