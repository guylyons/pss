//! The interactive view: a `less`-style pager with a selection bar.
//! Pinned header, scrolling, panning, `/` search, and `K` to kill the
//! selected process. The list refreshes itself while open. [`State`] holds
//! all the logic; [`run`] only draws it.

use std::io;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color as TColor, Modifier, Style as TStyle};
use ratatui::text::{Line as TLine, Span as TSpan};
use ratatui::widgets::Paragraph;
use ratatui::{DefaultTerminal, Frame};
use unicode_width::UnicodeWidthStr;

use crate::signal::Signal;
use crate::style::{Color, Role, theme};
use crate::table::{RowId, Table};
use crate::text::{Line, Matcher, highlight, plain_text};

/// What the view needs from the rest of the program.
pub trait Host {
    /// Collect the process list again, with the same options as before.
    fn reload(&mut self) -> io::Result<Table>;
    /// Send a signal; the error is a message for the status line.
    fn kill(&mut self, pid: i32, sig: Signal) -> Result<(), String>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
}

pub struct State {
    pub table: Table,
    texts: Vec<String>,
    /// Selected row.
    pub cursor: usize,
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
    /// Some while asking whether to kill this process.
    confirm: Option<RowId>,
    /// Sent a signal and still listed; dropped once a refresh shows them gone.
    dying: Vec<RowId>,
    pub message: Option<String>,
}

impl State {
    pub fn new(table: Table) -> State {
        State {
            texts: table.rows.iter().map(|l| plain_text(l)).collect(),
            table,
            cursor: 0,
            top: 0,
            left: 0,
            height: 1,
            width: 80,
            search: None,
            query: String::new(),
            prompt: None,
            confirm: None,
            dying: Vec::new(),
            message: None,
        }
    }

    pub fn selected(&self) -> Option<&RowId> {
        self.table.ids.get(self.cursor)
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        self.width = width.max(1);
        self.height = height.max(1);
        self.follow();
    }

    fn last(&self) -> usize {
        self.texts.len().saturating_sub(1)
    }

    fn max_top(&self) -> usize {
        self.texts.len().saturating_sub(self.height)
    }

    /// Scroll just enough to keep the selection on screen.
    fn follow(&mut self) {
        if self.cursor < self.top {
            self.top = self.cursor;
        } else if self.cursor >= self.top + self.height {
            self.top = self.cursor + 1 - self.height;
        }
        self.top = self.top.min(self.max_top());
    }

    fn move_cursor(&mut self, delta: isize) {
        self.cursor = self.cursor.saturating_add_signed(delta).min(self.last());
        self.follow();
    }

    /// Page: move the view and the selection together, like less.
    fn page(&mut self, delta: isize) {
        self.top = self.top.saturating_add_signed(delta).min(self.max_top());
        self.move_cursor(delta);
    }

    /// Swap in a fresh table, keeping the same process selected if it is
    /// still there, otherwise the same position.
    fn set_table(&mut self, table: Table) {
        let pid = self.selected().map(|r| r.pid);
        self.texts = table.rows.iter().map(|l| plain_text(l)).collect();
        self.table = table;
        self.cursor = pid
            .and_then(|pid| self.table.ids.iter().position(|r| r.pid == pid))
            .unwrap_or(self.cursor)
            .min(self.last());
        self.follow();
        let ids = &self.table.ids;
        let (alive, gone) = std::mem::take(&mut self.dying)
            .into_iter()
            .partition(|r| ids.contains(r));
        self.dying = alive;
        if let Some(RowId { pid, name }) = gone.last() {
            self.message = Some(format!("{pid} ({name}) exited"));
        }
    }

    /// Refresh on a timer, quicker while waiting for a killed process to exit.
    pub fn interval(&self) -> Duration {
        if self.dying.is_empty() {
            Duration::from_secs(2)
        } else {
            Duration::from_millis(250)
        }
    }

    pub fn tick(&mut self, host: &mut dyn Host) {
        if let Err(e) = self.reload(host) {
            self.message = Some(e);
        }
    }

    fn reload(&mut self, host: &mut dyn Host) -> Result<(), String> {
        let table = host.reload().map_err(|e| format!("Refresh failed: {e}"))?;
        self.set_table(table);
        Ok(())
    }

    pub fn key(&mut self, k: KeyEvent, host: &mut dyn Host) -> Flow {
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            return Flow::Quit;
        }
        if let Some(target) = self.confirm.take() {
            self.confirm_kill(target, k.code, host);
            return Flow::Continue;
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
            KeyCode::Char('j') | KeyCode::Down | KeyCode::Enter => self.move_cursor(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_cursor(-1),
            KeyCode::Char(' ') | KeyCode::Char('f') | KeyCode::PageDown => self.page(page),
            KeyCode::Char('b') | KeyCode::PageUp => self.page(-page),
            KeyCode::Char('d') => self.page(page / 2),
            KeyCode::Char('u') => self.page(-page / 2),
            KeyCode::Char('g') | KeyCode::Home => self.move_cursor(isize::MIN),
            KeyCode::Char('G') | KeyCode::End => self.move_cursor(isize::MAX),
            KeyCode::Right | KeyCode::Char('l') => self.left += pan,
            KeyCode::Left | KeyCode::Char('h') => self.left = self.left.saturating_sub(pan),
            KeyCode::Char('/') => self.prompt = Some(String::new()),
            KeyCode::Char('n') => self.jump(true, false),
            KeyCode::Char('N') => self.jump(false, false),
            KeyCode::Char('K') => match self.selected() {
                Some(row) => self.confirm = Some(row.clone()),
                None => self.message = Some("Nothing selected".into()),
            },
            KeyCode::Char('r') => self.tick(host),
            _ => {}
        }
        Flow::Continue
    }

    fn confirm_kill(&mut self, target: RowId, answer: KeyCode, host: &mut dyn Host) {
        let sig = match answer {
            KeyCode::Char('y') | KeyCode::Char('Y') => Signal::Term,
            KeyCode::Char('9') => Signal::Kill,
            _ => {
                self.message = Some("Kill cancelled".into());
                return;
            }
        };
        if let Err(e) = host.kill(target.pid, sig) {
            self.message = Some(e);
            return;
        }
        let RowId { pid, name } = &target;
        self.message = Some(format!("Sent {} to {pid} ({name}), waiting for it to exit", sig.name()));
        self.dying.push(target);
        self.tick(host);
    }

    /// Select the next (or previous) row matching the search. `inclusive`
    /// lets a fresh search land on the selected row itself.
    fn jump(&mut self, forward: bool, inclusive: bool) {
        let Some(m) = &self.search else {
            self.message = Some("No previous search".into());
            return;
        };
        let hit = |i: &usize| m.is_match(&self.texts[*i]);
        let found = if forward {
            let start = if inclusive {
                self.cursor
            } else {
                self.cursor + 1
            };
            (start..self.texts.len()).find(hit)
        } else {
            (0..self.cursor).rev().find(hit)
        };
        match found {
            Some(row) => {
                // Pan so the match is on screen.
                let at = m
                    .find(&self.texts[row])
                    .first()
                    .map_or(0, |r| self.texts[row][..r.0].width());
                if at < self.left || at >= self.left + self.width {
                    self.left = at.saturating_sub(self.width / 3);
                }
                self.cursor = row;
                self.follow();
            }
            None => self.message = Some(format!("Pattern not found: {}", self.query)),
        }
    }

    pub fn status(&self) -> String {
        if let Some(RowId { pid, name }) = &self.confirm {
            return format!("Kill {pid} ({name})?  y = TERM   9 = KILL   any other key cancels");
        }
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
            " rows {}-{bottom} of {n}  {pos}   ↑/↓ select  K kill  r refresh  / search  ←/→ pan  q quit",
            self.top + 1
        )
    }
}

pub fn run(table: Table, color: bool, host: &mut dyn Host) -> io::Result<()> {
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, State::new(table), color, host);
    ratatui::restore();
    result
}

fn event_loop(
    terminal: &mut DefaultTerminal,
    mut state: State,
    color: bool,
    host: &mut dyn Host,
) -> io::Result<()> {
    let mut refreshed = Instant::now();
    loop {
        terminal.draw(|f| draw(f, &mut state, color))?;
        // Count from the last refresh so steady typing can't hold it off.
        if !event::poll(state.interval().saturating_sub(refreshed.elapsed()))? {
            state.tick(host);
            refreshed = Instant::now();
            continue;
        }
        match event::read()? {
            Event::Key(k)
                if k.kind != KeyEventKind::Release && state.key(k, host) == Flow::Quit =>
            {
                return Ok(());
            }
            _ => {} // resize and others just redraw
        }
    }
}

fn draw(f: &mut Frame, state: &mut State, color: bool) {
    let [head, body, foot] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(f.area());
    state.resize(body.width as usize, body.height as usize);
    let left = state.left.min(u16::MAX as usize) as u16;

    f.render_widget(
        Paragraph::new(to_tui(&state.table.header, color)).scroll((0, left)),
        head,
    );

    let rows: Vec<TLine> = state
        .table
        .rows
        .iter()
        .skip(state.top)
        .take(state.height)
        .map(|line| match &state.search {
            Some(m) => to_tui(
                &highlight(line.clone(), &m.find(&plain_text(line)), Role::SearchHit),
                color,
            ),
            None => to_tui(line, color),
        })
        .collect();
    f.render_widget(Paragraph::new(rows).scroll((0, left)), body);

    // Row states span the full width, past the end of the text.
    let visible = state.table.ids.iter().enumerate().skip(state.top).take(state.height);
    for (i, id) in visible {
        let y = body.y + (i - state.top) as u16;
        let row = Rect::new(body.x, y, body.width, 1);
        if state.confirm.as_ref() == Some(id) {
            f.buffer_mut().set_style(row, tui_style(Role::Doomed, color));
            continue;
        }
        if state.dying.contains(id) {
            f.buffer_mut().set_style(row, tui_style(Role::Dying, color));
        }
        if i == state.cursor {
            f.buffer_mut().set_style(row, tui_style(Role::Selected, color));
            if color {
                // Default-colored text would be dark-on-dark on a light theme.
                for x in row.left()..row.right() {
                    let cell = &mut f.buffer_mut()[(x, y)];
                    if cell.fg == TColor::Reset {
                        cell.set_fg(TColor::Indexed(255));
                    }
                }
            }
        }
    }

    let status_role = if state.confirm.is_some() {
        Role::Doomed
    } else {
        Role::Selected
    };
    let status_style = if color {
        tui_style(status_role, true).fg(TColor::Indexed(255))
    } else {
        TStyle::default().add_modifier(Modifier::REVERSED)
    };
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
            Role::Match | Role::SearchHit | Role::Selected => {
                TStyle::default().add_modifier(Modifier::REVERSED)
            }
            Role::Doomed => TStyle::default().add_modifier(Modifier::REVERSED | Modifier::BOLD),
            Role::Dying => TStyle::default().add_modifier(Modifier::CROSSED_OUT),
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
        (s.strike, Modifier::CROSSED_OUT),
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
    use crate::table::RowId;
    use crate::text::Span;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn table_of(pids: &[i32]) -> Table {
        Table {
            header: vec![Span::new("PID CMD", Role::Header)],
            rows: pids
                .iter()
                .map(|p| {
                    let cmd = if *p == 1042 { "needle" } else { "hay" };
                    vec![Span::new(format!("{p} {cmd}"), Role::Plain)]
                })
                .collect(),
            ids: pids
                .iter()
                .map(|p| RowId {
                    pid: *p,
                    name: format!("p{}", p - 1000),
                })
                .collect(),
        }
    }

    fn pids(n: i32) -> Vec<i32> {
        (1000..1000 + n).collect()
    }

    /// Records signals instead of sending them; killed pids vanish on reload.
    #[derive(Default)]
    struct FakeHost {
        pids: Vec<i32>,
        kills: Vec<(i32, Signal)>,
        refuse: bool,
        /// Killed processes linger until the test removes them.
        slow: bool,
    }

    impl Host for FakeHost {
        fn reload(&mut self) -> io::Result<Table> {
            Ok(table_of(&self.pids))
        }
        fn kill(&mut self, pid: i32, sig: Signal) -> Result<(), String> {
            if self.refuse {
                return Err(format!("{pid}: not permitted"));
            }
            self.kills.push((pid, sig));
            if !self.slow {
                self.pids.retain(|p| *p != pid);
            }
            Ok(())
        }
    }

    fn setup(n: i32) -> (State, FakeHost) {
        let mut s = State::new(table_of(&pids(n)));
        s.resize(80, 10);
        (
            s,
            FakeHost {
                pids: pids(n),
                ..Default::default()
            },
        )
    }

    fn press(s: &mut State, h: &mut FakeHost, code: KeyCode) -> Flow {
        s.key(KeyEvent::new(code, KeyModifiers::NONE), h)
    }

    fn typed(s: &mut State, h: &mut FakeHost, text: &str) {
        for c in text.chars() {
            press(s, h, KeyCode::Char(c));
        }
    }

    fn selected_pid(s: &State) -> Option<i32> {
        s.selected().map(|r| r.pid)
    }

    #[test]
    fn cursor_moves_and_view_follows() {
        let (mut s, mut h) = setup(100);
        for _ in 0..12 {
            press(&mut s, &mut h, KeyCode::Down);
        }
        assert_eq!((s.cursor, s.top), (12, 3));
        press(&mut s, &mut h, KeyCode::Char('G'));
        assert_eq!((s.cursor, s.top), (99, 90));
        press(&mut s, &mut h, KeyCode::Char('k'));
        assert_eq!((s.cursor, s.top), (98, 90));
        press(&mut s, &mut h, KeyCode::Char('g'));
        assert_eq!((s.cursor, s.top), (0, 0));
        press(&mut s, &mut h, KeyCode::Up);
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn paging_moves_view_and_cursor_together() {
        let (mut s, mut h) = setup(100);
        press(&mut s, &mut h, KeyCode::Char('j'));
        press(&mut s, &mut h, KeyCode::Char(' '));
        assert_eq!((s.cursor, s.top), (11, 10));
        press(&mut s, &mut h, KeyCode::Char('b'));
        assert_eq!((s.cursor, s.top), (1, 0));
    }

    #[test]
    fn search_selects_the_match() {
        let (mut s, mut h) = setup(100);
        typed(&mut s, &mut h, "/needle");
        assert_eq!(s.status(), "/needle");
        press(&mut s, &mut h, KeyCode::Enter);
        assert_eq!(selected_pid(&s), Some(1042));
        assert!(s.top <= 42 && 42 < s.top + s.height);
        press(&mut s, &mut h, KeyCode::Char('n'));
        assert_eq!(s.message.as_deref(), Some("Pattern not found: needle"));
        press(&mut s, &mut h, KeyCode::Char('g'));
        press(&mut s, &mut h, KeyCode::Char('n'));
        assert_eq!(selected_pid(&s), Some(1042));
    }

    #[test]
    fn kill_asks_first_then_sends_term_and_refreshes() {
        let (mut s, mut h) = setup(10);
        typed(&mut s, &mut h, "jjj");
        press(&mut s, &mut h, KeyCode::Char('K'));
        assert!(s.status().starts_with("Kill 1003 (p3)?"), "{}", s.status());
        assert!(h.kills.is_empty(), "nothing sent before confirming");

        press(&mut s, &mut h, KeyCode::Char('y'));
        assert_eq!(h.kills, vec![(1003, Signal::Term)]);
        assert_eq!(s.status(), "1003 (p3) exited");
        assert!(!s.table.ids.iter().any(|r| r.pid == 1003), "list refreshed");
        assert_eq!(selected_pid(&s), Some(1004), "selection stays in place");
    }

    #[test]
    fn killed_process_is_marked_until_a_refresh_shows_it_gone() {
        let (mut s, mut h) = setup(10);
        h.slow = true;
        press(&mut s, &mut h, KeyCode::Char('K'));
        assert_eq!(s.confirm.as_ref().map(|r| r.pid), Some(1000), "about to die");
        press(&mut s, &mut h, KeyCode::Char('y'));
        assert_eq!(s.status(), "Sent TERM to 1000 (p0), waiting for it to exit");
        assert_eq!(s.dying.iter().map(|r| r.pid).collect::<Vec<_>>(), vec![1000]);
        assert_eq!(s.interval(), Duration::from_millis(250), "polls faster");

        s.tick(&mut h);
        assert_eq!(s.dying.len(), 1, "still running");
        h.pids.retain(|p| *p != 1000);
        s.tick(&mut h);
        assert!(s.dying.is_empty());
        assert!(!s.table.ids.iter().any(|r| r.pid == 1000), "removed");
        assert_eq!(s.status(), "1000 (p0) exited");
        assert_eq!(s.interval(), Duration::from_secs(2));
    }

    #[test]
    fn tick_picks_up_new_processes() {
        let (mut s, mut h) = setup(3);
        h.pids.push(2000);
        s.tick(&mut h);
        assert!(s.table.ids.iter().any(|r| r.pid == 2000));
    }

    #[test]
    fn nine_sends_kill() {
        let (mut s, mut h) = setup(10);
        press(&mut s, &mut h, KeyCode::Char('K'));
        press(&mut s, &mut h, KeyCode::Char('9'));
        assert_eq!(h.kills, vec![(1000, Signal::Kill)]);
    }

    #[test]
    fn any_other_key_cancels_kill() {
        let (mut s, mut h) = setup(10);
        press(&mut s, &mut h, KeyCode::Char('K'));
        assert_eq!(press(&mut s, &mut h, KeyCode::Char('q')), Flow::Continue);
        assert!(h.kills.is_empty());
        assert_eq!(s.status(), "Kill cancelled");
    }

    #[test]
    fn kill_failure_is_reported() {
        let (mut s, mut h) = setup(10);
        h.refuse = true;
        press(&mut s, &mut h, KeyCode::Char('K'));
        press(&mut s, &mut h, KeyCode::Char('y'));
        assert_eq!(s.status(), "1000: not permitted");
        assert_eq!(selected_pid(&s), Some(1000));
    }

    #[test]
    fn refresh_keeps_the_selected_process() {
        let (mut s, mut h) = setup(10);
        typed(&mut s, &mut h, "jjjjj");
        h.pids.retain(|p| *p != 1001);
        press(&mut s, &mut h, KeyCode::Char('r'));
        assert_eq!(selected_pid(&s), Some(1005));
        assert_eq!(s.cursor, 4);
    }

    #[test]
    fn selection_clamps_when_the_list_shrinks() {
        let (mut s, mut h) = setup(10);
        press(&mut s, &mut h, KeyCode::Char('G'));
        h.pids.truncate(3);
        press(&mut s, &mut h, KeyCode::Char('r'));
        assert_eq!(selected_pid(&s), Some(1002));
    }

    #[test]
    fn kill_with_nothing_listed() {
        let mut s = State::new(table_of(&[]));
        let mut h = FakeHost::default();
        press(&mut s, &mut h, KeyCode::Char('K'));
        assert_eq!(s.status(), "Nothing selected");
    }

    #[test]
    fn escape_cancels_prompt_then_quits() {
        let (mut s, mut h) = setup(5);
        typed(&mut s, &mut h, "/x");
        assert_eq!(press(&mut s, &mut h, KeyCode::Esc), Flow::Continue);
        assert!(s.prompt.is_none());
        assert_eq!(press(&mut s, &mut h, KeyCode::Char('q')), Flow::Quit);
    }

    #[test]
    fn short_output_shows_end() {
        let (s, _) = setup(3);
        assert!(s.status().contains("(END)"));
    }

    #[test]
    fn selected_row_is_drawn_as_a_bar() {
        let (mut s, mut h) = setup(10);
        typed(&mut s, &mut h, "jj");
        let mut term = Terminal::new(TestBackend::new(30, 8)).unwrap();
        term.draw(|f| draw(f, &mut s, true)).unwrap();
        let buf = term.backend().buffer();
        // Row 0 is the header, so the third body row is y = 3.
        for x in 0..30 {
            assert_eq!(buf[(x, 3)].bg, TColor::Indexed(237), "x={x}");
        }
        assert_eq!(buf[(0, 2)].bg, TColor::Reset);
    }

    #[test]
    fn kill_states_color_their_rows() {
        let (mut s, mut h) = setup(10);
        h.slow = true;
        press(&mut s, &mut h, KeyCode::Char('K'));
        press(&mut s, &mut h, KeyCode::Char('y'));
        press(&mut s, &mut h, KeyCode::Char('j'));
        press(&mut s, &mut h, KeyCode::Char('K'));
        let mut term = Terminal::new(TestBackend::new(30, 8)).unwrap();
        term.draw(|f| draw(f, &mut s, true)).unwrap();
        let buf = term.backend().buffer();
        assert!(buf[(29, 1)].modifier.contains(Modifier::CROSSED_OUT), "dying");
        assert_eq!(buf[(29, 2)].bg, TColor::Indexed(124), "about to die");
        assert_eq!(buf[(0, 7)].bg, TColor::Indexed(124), "status asks in red");
    }
}
