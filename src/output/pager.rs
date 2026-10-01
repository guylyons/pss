//! A `less`-style viewer: pinned header, scrolling, horizontal panning and
//! `/` search. [`State`] holds all the logic; [`run`] only draws it.

use std::io;

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color as TColor, Modifier, Style as TStyle};
use ratatui::text::{Line as TLine, Span as TSpan};
use ratatui::widgets::Paragraph;
use ratatui::{DefaultTerminal, Frame};
use unicode_width::UnicodeWidthStr;

use crate::style::{Color, Role, theme};
use crate::table::Table;
use crate::text::{Line, Matcher, highlight, plain_text};

#[derive(Debug, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
}

pub struct State {
    texts: Vec<String>,
    /// First visible row and leftmost visible column.
    pub top: usize,
    pub left: usize,
    /// Body rows and columns on screen.
    pub height: usize,
    pub width: usize,
    search: Option<Matcher>,
    query: String,
    /// Some while typing a `/` query.
    pub prompt: Option<String>,
    pub message: Option<String>,
}

impl State {
    pub fn new(table: &Table) -> State {
        State {
            texts: table.rows.iter().map(|l| plain_text(l)).collect(),
            top: 0,
            left: 0,
            height: 1,
            width: 80,
            search: None,
            query: String::new(),
            prompt: None,
            message: None,
        }
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        self.width = width.max(1);
        self.height = height.max(1);
        self.top = self.top.min(self.max_top());
    }

    fn max_top(&self) -> usize {
        self.texts.len().saturating_sub(self.height)
    }

    fn scroll(&mut self, delta: isize) {
        self.top = self.top.saturating_add_signed(delta).min(self.max_top());
    }

    pub fn key(&mut self, k: KeyEvent) -> Flow {
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            return Flow::Quit;
        }
        if let Some(input) = self.prompt.as_mut() {
            match k.code {
                KeyCode::Enter => {
                    let q = self.prompt.take().unwrap_or_default();
                    if !q.is_empty() {
                        self.query = q;
                        self.search = Matcher::new(&[&self.query]);
                    }
                    self.jump(true, true);
                }
                KeyCode::Esc => self.prompt = None,
                KeyCode::Backspace => {
                    if input.pop().is_none() {
                        self.prompt = None;
                    }
                }
                KeyCode::Char(c) => input.push(c),
                _ => {}
            }
            return Flow::Continue;
        }

        self.message = None;
        let page = self.height as isize;
        let pan = (self.width / 2).max(1);
        match k.code {
            KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => return Flow::Quit,
            KeyCode::Char('j') | KeyCode::Down | KeyCode::Enter => self.scroll(1),
            KeyCode::Char('k') | KeyCode::Up => self.scroll(-1),
            KeyCode::Char(' ') | KeyCode::Char('f') | KeyCode::PageDown => self.scroll(page),
            KeyCode::Char('b') | KeyCode::PageUp => self.scroll(-page),
            KeyCode::Char('d') => self.scroll(page / 2),
            KeyCode::Char('u') => self.scroll(-page / 2),
            KeyCode::Char('g') | KeyCode::Home => self.top = 0,
            KeyCode::Char('G') | KeyCode::End => self.top = self.max_top(),
            KeyCode::Right | KeyCode::Char('l') => self.left += pan,
            KeyCode::Left | KeyCode::Char('h') => self.left = self.left.saturating_sub(pan),
            KeyCode::Char('/') => self.prompt = Some(String::new()),
            KeyCode::Char('n') => self.jump(true, false),
            KeyCode::Char('N') => self.jump(false, false),
            _ => {}
        }
        Flow::Continue
    }

    /// Move to the next (or previous) row matching the search. `inclusive`
    /// lets a fresh search land on the current top row.
    fn jump(&mut self, forward: bool, inclusive: bool) {
        let Some(m) = &self.search else {
            self.message = Some("No previous search".into());
            return;
        };
        let hit = |i: &usize| m.is_match(&self.texts[*i]);
        let found = if forward {
            let start = if inclusive { self.top } else { self.top + 1 };
            (start..self.texts.len()).find(hit)
        } else {
            (0..self.top).rev().find(hit)
        };
        match found {
            Some(row) => {
                self.top = row.min(self.max_top());
                // Pan so the match is on screen.
                let at = m
                    .find(&self.texts[row])
                    .first()
                    .map_or(0, |r| self.texts[row][..r.0].width());
                if at < self.left || at >= self.left + self.width {
                    self.left = at.saturating_sub(self.width / 3);
                }
            }
            None => self.message = Some(format!("Pattern not found: {}", self.query)),
        }
    }

    pub fn status(&self) -> String {
        if let Some(p) = &self.prompt {
            return format!("/{p}");
        }
        if let Some(m) = &self.message {
            return m.clone();
        }
        let n = self.texts.len();
        let bottom = (self.top + self.height).min(n);
        let pos = if bottom >= n {
            "(END)".to_string()
        } else {
            format!("{}%", bottom * 100 / n.max(1))
        };
        format!(
            " rows {}-{bottom} of {n}  {pos}   q quit  / search  ←/→ pan",
            self.top + 1
        )
    }

    fn search(&self) -> Option<&Matcher> {
        self.search.as_ref()
    }
}

pub fn run(table: &Table, color: bool) -> io::Result<()> {
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, table, color);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut DefaultTerminal, table: &Table, color: bool) -> io::Result<()> {
    let mut state = State::new(table);
    loop {
        terminal.draw(|f| draw(f, table, &mut state, color))?;
        match event::read()? {
            Event::Key(k) if k.kind != KeyEventKind::Release && state.key(k) == Flow::Quit => {
                return Ok(());
            }
            _ => {} // resize and others just redraw
        }
    }
}

fn draw(f: &mut Frame, table: &Table, state: &mut State, color: bool) {
    let [head, body, foot] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(f.area());
    state.resize(body.width as usize, body.height as usize);
    let left = state.left.min(u16::MAX as usize) as u16;

    f.render_widget(
        Paragraph::new(to_tui(&table.header, color)).scroll((0, left)),
        head,
    );

    let rows: Vec<TLine> = table
        .rows
        .iter()
        .skip(state.top)
        .take(state.height)
        .map(|line| match state.search() {
            Some(m) => to_tui(
                &highlight(line.clone(), &m.find(&plain_text(line)), Role::SearchHit),
                color,
            ),
            None => to_tui(line, color),
        })
        .collect();
    f.render_widget(Paragraph::new(rows).scroll((0, left)), body);

    let status_style = TStyle::default().add_modifier(Modifier::REVERSED);
    f.render_widget(Paragraph::new(state.status()).style(status_style), foot);
}

fn to_tui(line: &Line, color: bool) -> TLine<'static> {
    TLine::from(
        line.iter()
            .map(|s| TSpan::styled(s.text.clone(), tui_style(s.role, color)))
            .collect::<Vec<_>>(),
    )
}

fn tui_style(role: Role, color: bool) -> TStyle {
    if !color {
        // Without color, still make headers and matches findable.
        return match role {
            Role::Header => TStyle::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            Role::Match | Role::SearchHit => TStyle::default().add_modifier(Modifier::REVERSED),
            _ => TStyle::default(),
        };
    }
    let s = theme(role);
    let mut t = TStyle::default();
    if let Some(c) = s.fg {
        t = t.fg(tui_color(c));
    }
    if let Some(c) = s.bg {
        t = t.bg(tui_color(c));
    }
    for (on, m) in [
        (s.bold, Modifier::BOLD),
        (s.dim, Modifier::DIM),
        (s.underline, Modifier::UNDERLINED),
    ] {
        if on {
            t = t.add_modifier(m);
        }
    }
    t
}

fn tui_color(c: Color) -> TColor {
    match c {
        Color::Black => TColor::Black,
        Color::Red => TColor::Red,
        Color::Green => TColor::Green,
        Color::Yellow => TColor::Yellow,
        Color::Blue => TColor::Blue,
        Color::Magenta => TColor::Magenta,
        Color::Cyan => TColor::Cyan,
        Color::Indexed(n) => TColor::Indexed(n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::Span;

    fn table(n: usize) -> Table {
        Table {
            header: vec![Span::new("PID CMD", Role::Header)],
            rows: (0..n)
                .map(|i| {
                    vec![Span::new(
                        format!("{i} {}", if i == 42 { "needle" } else { "hay" }),
                        Role::Plain,
                    )]
                })
                .collect(),
        }
    }

    fn press(s: &mut State, code: KeyCode) -> Flow {
        s.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn typed(s: &mut State, text: &str) {
        for c in text.chars() {
            press(s, KeyCode::Char(c));
        }
    }

    #[test]
    fn scrolling_clamps_to_last_page() {
        let mut s = State::new(&table(100));
        s.resize(80, 10);
        press(&mut s, KeyCode::Char('G'));
        assert_eq!(s.top, 90);
        press(&mut s, KeyCode::Char(' '));
        assert_eq!(s.top, 90);
        press(&mut s, KeyCode::Char('b'));
        assert_eq!(s.top, 80);
        press(&mut s, KeyCode::Char('g'));
        press(&mut s, KeyCode::Char('k'));
        assert_eq!(s.top, 0);
    }

    #[test]
    fn search_jumps_and_reports_misses() {
        let mut s = State::new(&table(100));
        s.resize(80, 10);
        typed(&mut s, "/needle");
        assert_eq!(s.status(), "/needle");
        press(&mut s, KeyCode::Enter);
        assert_eq!(s.top, 42);
        press(&mut s, KeyCode::Char('n'));
        assert_eq!(s.message.as_deref(), Some("Pattern not found: needle"));
        press(&mut s, KeyCode::Char('g'));
        press(&mut s, KeyCode::Char('n'));
        assert_eq!(s.top, 42);
    }

    #[test]
    fn escape_cancels_prompt_then_quits() {
        let mut s = State::new(&table(5));
        typed(&mut s, "/x");
        assert_eq!(press(&mut s, KeyCode::Esc), Flow::Continue);
        assert!(s.prompt.is_none());
        assert_eq!(press(&mut s, KeyCode::Char('q')), Flow::Quit);
    }

    #[test]
    fn short_output_shows_end() {
        let mut s = State::new(&table(3));
        s.resize(80, 10);
        assert!(s.status().contains("(END)"));
    }
}
