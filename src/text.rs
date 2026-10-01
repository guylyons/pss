//! Styled text shared by the plain printer and the pager.

use unicode_width::UnicodeWidthChar;

use crate::style::Role;

#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub text: String,
    pub role: Role,
}

impl Span {
    pub fn new(text: impl Into<String>, role: Role) -> Span {
        Span {
            text: text.into(),
            role,
        }
    }
}

pub type Line = Vec<Span>;

pub fn plain_text(line: &[Span]) -> String {
    line.iter().map(|s| s.text.as_str()).collect()
}

/// Re-tag the byte ranges `ranges` (sorted, non-overlapping, measured over
/// the line's concatenated text) with `role`, splitting spans as needed.
pub fn highlight(line: Line, ranges: &[(usize, usize)], role: Role) -> Line {
    if ranges.is_empty() {
        return line;
    }
    let mut out = Vec::with_capacity(line.len() + ranges.len() * 2);
    let mut offset = 0;
    for span in line {
        let (start, end) = (offset, offset + span.text.len());
        offset = end;
        let mut cursor = start;
        for &(rs, re) in ranges {
            let (s, e) = (rs.max(cursor), re.min(end));
            if s >= e {
                continue;
            }
            if s > cursor {
                out.push(Span::new(&span.text[cursor - start..s - start], span.role));
            }
            out.push(Span::new(&span.text[s - start..e - start], role));
            cursor = e;
        }
        if cursor < end {
            out.push(Span::new(&span.text[cursor - start..], span.role));
        }
    }
    out
}

/// Cut a line to at most `width` terminal columns.
pub fn truncate(line: Line, width: usize) -> Line {
    let mut used = 0;
    let mut out = Vec::with_capacity(line.len());
    for span in line {
        let mut cut = None;
        for (i, c) in span.text.char_indices() {
            let w = c.width().unwrap_or(0);
            if used + w > width {
                cut = Some(i);
                break;
            }
            used += w;
        }
        match cut {
            None => out.push(span),
            Some(i) => {
                if i > 0 {
                    out.push(Span::new(&span.text[..i], span.role));
                }
                return out;
            }
        }
    }
    out
}

/// Substring search where an all-lowercase pattern ignores case ("smart case").
/// Case folding is ASCII-only so byte offsets stay valid for highlighting.
#[derive(Debug, Clone)]
pub struct Matcher {
    needles: Vec<(String, bool)>, // (needle, case_insensitive)
}

impl Matcher {
    pub fn new<S: AsRef<str>>(patterns: &[S]) -> Option<Matcher> {
        let needles: Vec<_> = patterns
            .iter()
            .map(|p| p.as_ref())
            .filter(|p| !p.is_empty())
            .map(|p| {
                let insensitive = !p.chars().any(|c| c.is_uppercase());
                let n = if insensitive {
                    p.to_ascii_lowercase()
                } else {
                    p.to_string()
                };
                (n, insensitive)
            })
            .collect();
        (!needles.is_empty()).then_some(Matcher { needles })
    }

    pub fn is_match(&self, hay: &str) -> bool {
        !self.find(hay).is_empty()
    }

    /// All match ranges, sorted and merged.
    pub fn find(&self, hay: &str) -> Vec<(usize, usize)> {
        let lower = hay.to_ascii_lowercase();
        let mut ranges = Vec::new();
        for (needle, insensitive) in &self.needles {
            let h = if *insensitive { &lower } else { hay };
            ranges.extend(
                h.match_indices(needle.as_str())
                    .map(|(i, m)| (i, i + m.len())),
            );
        }
        ranges.sort_unstable();
        let mut merged: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
        for (s, e) in ranges {
            match merged.last_mut() {
                Some(last) if s <= last.1 => last.1 = last.1.max(e),
                _ => merged.push((s, e)),
            }
        }
        merged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(parts: &[(&str, Role)]) -> Line {
        parts.iter().map(|(t, r)| Span::new(*t, *r)).collect()
    }

    #[test]
    fn highlight_splits_across_spans() {
        let line = spans(&[
            ("/usr/bin/", Role::CmdDir),
            ("node", Role::CmdExe),
            (" app.js", Role::CmdArg),
        ]);
        let out = highlight(line, &[(7, 11)], Role::Match);
        assert_eq!(
            out,
            spans(&[
                ("/usr/bi", Role::CmdDir),
                ("n/", Role::Match),
                ("no", Role::Match),
                ("de", Role::CmdExe),
                (" app.js", Role::CmdArg)
            ])
        );
    }

    #[test]
    fn smart_case() {
        let m = Matcher::new(&["node"]).unwrap();
        assert_eq!(m.find("Node and node"), vec![(0, 4), (9, 13)]);
        let m = Matcher::new(&["Node"]).unwrap();
        assert_eq!(m.find("Node and node"), vec![(0, 4)]);
    }

    #[test]
    fn overlapping_patterns_merge() {
        let m = Matcher::new(&["abc", "bcd"]).unwrap();
        assert_eq!(m.find("xabcde"), vec![(1, 5)]);
        assert!(Matcher::new::<&str>(&[]).is_none());
    }

    #[test]
    fn truncate_respects_wide_chars() {
        let out = truncate(spans(&[("ab", Role::Plain), ("日本語", Role::CmdArg)]), 5);
        assert_eq!(plain_text(&out), "ab日");
    }
}
