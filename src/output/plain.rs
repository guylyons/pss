//! Printing a table straight to a stream, with or without ANSI color.

use std::io::{self, Write};

use crate::style::{Color, Style, theme};
use crate::table::Table;
use crate::text::{Line, truncate};

/// `width`: cut lines to this many columns (terminal output without `-ww`).
pub fn write(
    out: &mut impl Write,
    table: &Table,
    color: bool,
    width: Option<usize>,
) -> io::Result<()> {
    let header = (!table.header.is_empty()).then_some(&table.header);
    for line in header.into_iter().chain(&table.rows) {
        let line = match width {
            Some(w) => truncate(line.clone(), w),
            None => line.clone(),
        };
        write_line(out, &line, color)?;
    }
    out.flush()
}

fn write_line(out: &mut impl Write, line: &Line, color: bool) -> io::Result<()> {
    for span in line.iter().filter(|s| !s.text.is_empty()) {
        let sgr = if color {
            sgr(theme(span.role))
        } else {
            String::new()
        };
        if sgr.is_empty() {
            out.write_all(span.text.as_bytes())?;
        } else {
            write!(out, "\x1b[{sgr}m{}\x1b[0m", span.text)?;
        }
    }
    out.write_all(b"\n")
}

/// SGR parameters for a style, e.g. `1;31`. Empty for the default style.
pub fn sgr(s: Style) -> String {
    let mut parts: Vec<String> = Vec::new();
    if s.bold {
        parts.push("1".into());
    }
    if s.dim {
        parts.push("2".into());
    }
    if s.underline {
        parts.push("4".into());
    }
    if let Some(c) = s.fg {
        parts.push(color_code(c, 30));
    }
    if let Some(c) = s.bg {
        parts.push(color_code(c, 40));
    }
    parts.join(";")
}

fn color_code(c: Color, base: u8) -> String {
    let basic = |n: u8| (base + n).to_string();
    match c {
        Color::Black => basic(0),
        Color::Red => basic(1),
        Color::Green => basic(2),
        Color::Yellow => basic(3),
        Color::Blue => basic(4),
        Color::Magenta => basic(5),
        Color::Cyan => basic(6),
        Color::Indexed(n) => format!("{};5;{n}", base + 8),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::Role;
    use crate::text::Span;

    fn table() -> Table {
        Table {
            header: vec![Span::new("PID", Role::Header)],
            rows: vec![vec![
                Span::new("1 ", Role::Plain),
                Span::new("90.0", Role::Pcpu(90.0)),
            ]],
            ids: vec![],
        }
    }

    #[test]
    fn plain_has_no_escapes() {
        let mut out = Vec::new();
        write(&mut out, &table(), false, None).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "PID\n1 90.0\n");
    }

    #[test]
    fn color_wraps_styled_spans_only() {
        let mut out = Vec::new();
        write(&mut out, &table(), true, Some(4)).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "\x1b[1;4mPID\x1b[0m\n1 \x1b[1;31m90\x1b[0m\n"
        );
    }

    #[test]
    fn indexed_colors() {
        assert_eq!(
            sgr(Style {
                fg: Some(Color::Indexed(208)),
                ..Style::default()
            }),
            "38;5;208"
        );
    }
}
